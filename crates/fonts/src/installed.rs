//! Fonts installed on this machine, used automatically for edited text the PDF's own fonts can't show.
//!
//! The first lookup scans the standard font folders once (bounded: file count, file size and total
//! bytes read), keeps each usable face's path, names and Unicode coverage, and drops the bytes. A
//! face is usable when [`crate::embedding::validate`] accepts it: a standalone, static
//! TrueType/OpenType font whose `fsType` allows editable embedding. Collections (`.ttc`) and
//! variable fonts are skipped, because the embedder can't write them.
//!
//! `PDFCRAFT_SYSTEM_FONTS=0` turns this off, as it does the interface fallback.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

/// Largest font file read (the embedder's own limit).
const MAX_FILE: u64 = 16 << 20;
/// Total bytes read while building the index.
const MAX_SCAN_BYTES: u64 = 2 << 30;
/// Directory entries visited while building the index.
const MAX_ENTRIES: usize = 16_384;
/// Directory depth below each root.
const MAX_DEPTH: usize = 5;
/// Font programs kept in memory after a lookup, so typing doesn't re-read the file per keystroke.
const KEEP_LOADED: usize = 4;

/// The broad look of a face, matched against the paragraph being edited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Sans,
    Serif,
    Mono,
}

/// One usable installed face.
#[derive(Clone, Debug)]
pub struct InstalledFace {
    pub path: PathBuf,
    pub family: String,
    pub postscript: String,
    pub bold: bool,
    pub italic: bool,
    pub class: Class,
    /// Sorted, disjoint inclusive code point ranges the face maps to a real glyph.
    ranges: Vec<(u32, u32)>,
}

impl InstalledFace {
    /// Whether the face has a glyph for `c`.
    pub fn has(&self, c: char) -> bool {
        let c = u32::from(c);
        let at = self.ranges.partition_point(|(_, end)| *end < c);
        self.ranges.get(at).is_some_and(|(start, _)| *start <= c)
    }

    /// Whether the face has a glyph for every character of `text` but line breaks.
    pub fn covers(&self, text: &str) -> bool {
        text.chars().filter(|c| !matches!(c, '\n' | '\r')).all(|c| self.has(c))
    }
}

fn enabled() -> bool {
    std::env::var_os("PDFCRAFT_SYSTEM_FONTS").is_none_or(|v| v != "0")
}

/// The standard font folders of this platform, system ones included.
pub fn directories() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        let mut dirs = vec![PathBuf::from("/System/Library/Fonts"), PathBuf::from("/Library/Fonts")];
        dirs.extend(home.map(|h| h.join("Library/Fonts")));
        dirs
    } else if cfg!(windows) {
        let windir =
            std::env::var_os("WINDIR").or_else(|| std::env::var_os("SystemRoot")).map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
        let mut dirs = vec![windir.join("Fonts")];
        dirs.extend(std::env::var_os("LOCALAPPDATA").map(|l| PathBuf::from(l).join("Microsoft/Windows/Fonts")));
        dirs
    } else {
        let mut dirs = vec![PathBuf::from("/usr/share/fonts"), PathBuf::from("/usr/local/share/fonts")];
        if let Some(home) = home {
            dirs.extend([home.join(".local/share/fonts"), home.join(".fonts")]);
        }
        dirs
    }
}

/// Every usable installed face, scanned once per process. Empty when turned off or on the web.
pub fn faces() -> &'static [InstalledFace] {
    static INDEX: OnceLock<Vec<InstalledFace>> = OnceLock::new();
    INDEX.get_or_init(|| if enabled() && !cfg!(target_arch = "wasm32") { scan(&directories()) } else { Vec::new() })
}

/// Build the index on a background thread, so the first edit that needs it doesn't wait.
pub fn warm_up() {
    if !cfg!(target_arch = "wasm32") {
        let _ = std::thread::Builder::new().name("installed-font-index".into()).spawn(|| {
            let _ = faces();
        });
    }
}

/// Bounded, symlink-free scan of `roots`, sorted by family then style name.
pub fn scan(roots: &[PathBuf]) -> Vec<InstalledFace> {
    let mut pending: Vec<(PathBuf, usize)> = roots.iter().take(16).map(|r| (r.clone(), 0)).collect();
    let mut seen = std::collections::HashSet::new();
    let mut faces = Vec::new();
    let mut entries = 0usize;
    let mut budget = MAX_SCAN_BYTES;
    while let Some((path, depth)) = pending.pop() {
        if entries >= MAX_ENTRIES || budget == 0 {
            break;
        }
        entries = entries.saturating_add(1);
        let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
        if meta.file_type().is_symlink() || !seen.insert(path.clone()) {
            continue;
        }
        if meta.is_dir() {
            if depth < MAX_DEPTH
                && let Ok(children) = std::fs::read_dir(&path)
            {
                let room = MAX_ENTRIES.saturating_sub(entries.saturating_add(pending.len()));
                pending.extend(children.take(room).filter_map(Result::ok).map(|e| (e.path(), depth.saturating_add(1))));
            }
            continue;
        }
        if !meta.is_file() || meta.len() > MAX_FILE || meta.len() > budget || !font_extension(&path) {
            continue;
        }
        budget = budget.saturating_sub(meta.len());
        if let Some(face) = read_capped(&path).and_then(|bytes| index_face(&path, &bytes)) {
            faces.push(face);
        }
    }
    faces.sort_by(|a, b| a.family.cmp(&b.family).then(a.postscript.cmp(&b.postscript)).then(a.path.cmp(&b.path)));
    faces
}

fn font_extension(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("ttf") || e.eq_ignore_ascii_case("otf"))
}

/// The file's bytes when it is at most [`MAX_FILE`], with the read itself capped.
fn read_capped(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    std::fs::File::open(path).ok()?.take(MAX_FILE.saturating_add(1)).read_to_end(&mut bytes).ok()?;
    (u64::try_from(bytes.len()).ok()? <= MAX_FILE).then_some(bytes)
}

fn index_face(path: &Path, bytes: &[u8]) -> Option<InstalledFace> {
    use skrifa::MetadataProvider as _;
    use skrifa::raw::TableProvider as _;
    // Names, structure and embedding permission, as the embedder checks them.
    let (family, postscript) = crate::embedding::names(bytes).ok()?;
    let font = skrifa::FontRef::new(bytes).ok()?;
    let mut ranges: Vec<(u32, u32)> = Vec::new();
    for (cp, glyph) in font.charmap().mappings() {
        if glyph.to_u32() == 0 {
            continue;
        }
        match ranges.last_mut() {
            Some((_, end)) if end.checked_add(1) == Some(cp) => *end = cp,
            _ => ranges.push((cp, cp)),
        }
    }
    // The charmap iterates in code point order, but don't rely on it for the binary search.
    ranges.sort_unstable();
    let (weight, italic) = font.os2().map_or((400, false), |t| (t.us_weight_class(), t.fs_selection().bits() & 0x0001 != 0));
    let lower = format!("{family} {postscript}").to_ascii_lowercase();
    let bold = weight >= 600 || lower.contains("bold");
    let italic = italic || lower.contains("italic") || lower.contains("oblique");
    Some(InstalledFace { path: path.to_owned(), class: class_of(&lower), family, postscript, bold, italic, ranges })
}

fn class_of(name: &str) -> Class {
    if ["mono", "courier", "consolas", "menlo", "monaco", "console", "code"].iter().any(|s| name.contains(s)) {
        Class::Mono
    } else if !name.contains("sans")
        && ["serif", "times", "roman", "mincho", "georgia", "garamond", "palatino", "cambria", "naskh"].iter().any(|s| name.contains(s))
    {
        Class::Serif
    } else {
        Class::Sans
    }
}

/// Families that read as ordinary document text, preferred over decorative faces that happen to
/// cover the same characters (a calligraphic Arabic face, say).
const COMMON: [&str; 16] = [
    "Arial",
    "Helvetica",
    "Segoe UI",
    "Tahoma",
    "Noto Sans",
    "DejaVu Sans",
    "Liberation Sans",
    "Verdana",
    "Calibri",
    "Microsoft Sans Serif",
    "Times New Roman",
    "Noto Serif",
    "DejaVu Serif",
    "Liberation Serif",
    "Courier New",
    "DejaVu Sans Mono",
];

/// The best of `faces` for `text`: one that covers all of it, the paragraph's class and weight
/// first, then common document families, then the rest by name. `None` when no face covers it.
pub fn best<'a>(faces: &'a [InstalledFace], text: &str, class: Class, bold: bool, italic: bool) -> Option<&'a InstalledFace> {
    let common = |f: &InstalledFace| {
        COMMON.iter().position(|c| f.family.eq_ignore_ascii_case(c) || f.family.starts_with(&format!("{c} "))).unwrap_or(COMMON.len())
    };
    faces.iter().filter(|f| f.covers(text)).min_by_key(|f| (f.class != class, f.bold != bold, f.italic != italic, common(f), f.family.len()))
}

/// The program of `face`, re-read and validated (the file may have changed since the scan).
/// Recently used programs are kept, so a lookup per keystroke reads the file once.
pub fn load(face: &InstalledFace) -> Option<Arc<Vec<u8>>> {
    static LOADED: Mutex<Vec<(PathBuf, Arc<Vec<u8>>)>> = Mutex::new(Vec::new());
    let mut loaded = LOADED.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(at) = loaded.iter().position(|(p, _)| *p == face.path) {
        let entry = loaded.remove(at);
        let bytes = Arc::clone(&entry.1);
        loaded.push(entry);
        return Some(bytes);
    }
    let bytes = read_capped(&face.path)?;
    crate::embedding::validate(&bytes).ok()?;
    let bytes = Arc::new(bytes);
    if loaded.len() >= KEEP_LOADED {
        loaded.remove(0);
    }
    loaded.push((face.path.clone(), Arc::clone(&bytes)));
    Some(bytes)
}

/// An installed font program that can show all of `text` in the paragraph's look, with its
/// PostScript name. `None` when turned off or nothing installed covers it.
pub fn font_for(text: &str, class: Class, bold: bool, italic: bool) -> Option<(Arc<Vec<u8>>, String)> {
    let face = best(faces(), text, class, bold, italic)?;
    Some((load(face)?, face.postscript.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets() -> PathBuf {
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts"))
    }

    fn face(family: &str, class: Class, bold: bool, ranges: Vec<(u32, u32)>) -> InstalledFace {
        InstalledFace { path: PathBuf::from(family), family: family.into(), postscript: family.replace(' ', ""), bold, italic: false, class, ranges }
    }

    #[test]
    fn the_scan_indexes_standalone_fonts_and_their_coverage() {
        let faces = scan(&[assets()]);
        let inter = faces.iter().find(|f| f.family == "Inter").expect("Inter is in assets/fonts");
        assert!(inter.covers("PdfCraft Жж Ωω"));
        assert!(!inter.has('\u{0635}'), "Inter has no Arabic");
        assert!(!inter.covers("ص"));
        assert_eq!(inter.class, Class::Sans);
        assert!(faces.iter().any(|f| f.family == "JetBrains Mono" && f.class == Class::Mono));
        // Ranges are sorted and disjoint, so the binary search in `has` is sound.
        for f in &faces {
            assert!(f.ranges.windows(2).all(|w| w[0].1 < w[1].0), "{}", f.family);
        }
    }

    #[test]
    fn the_scan_skips_what_isnt_a_usable_font() {
        let dir = std::env::temp_dir().join(format!("pdfcraft-installed-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(dir.join("broken.ttf"), b"not a font").expect("write");
        std::fs::write(dir.join("empty.otf"), b"").expect("write");
        assert!(scan(&[dir.clone(), PathBuf::from("definitely/not/here")]).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_best_face_covers_everything_and_matches_the_paragraph() {
        let arabic = (0x0600, 0x06FF);
        let latin = (0x20, 0x7E);
        let faces = [
            face("Diwan Thuluth", Class::Sans, false, vec![latin, arabic]),
            face("Arial", Class::Sans, false, vec![latin, arabic]),
            face("Arial Bold", Class::Sans, true, vec![latin, arabic]),
            face("Times New Roman", Class::Serif, false, vec![latin, arabic]),
            face("Inter", Class::Sans, false, vec![latin]),
        ];
        let pick = |text, class, bold| best(&faces, text, class, bold, false).map(|f| f.family.as_str());
        assert_eq!(pick("مرحبا world", Class::Sans, false), Some("Arial"), "a common family over a decorative one");
        assert_eq!(pick("مرحبا", Class::Sans, true), Some("Arial Bold"));
        assert_eq!(pick("مرحبا", Class::Serif, false), Some("Times New Roman"));
        assert_eq!(pick("hello", Class::Sans, false), Some("Arial"));
        assert_eq!(pick("日本", Class::Sans, false), None, "nothing covers it");
        assert_eq!(best(&[], "a", Class::Sans, false, false).map(|f| f.family.as_str()), None);
    }

    #[test]
    fn a_loaded_program_is_validated_and_reused() {
        let faces = scan(&[assets()]);
        let inter = faces.iter().find(|f| f.family == "Inter").expect("Inter");
        let first = load(inter).expect("Inter loads");
        let again = load(inter).expect("Inter loads again");
        assert!(Arc::ptr_eq(&first, &again), "the program is read once");
        let gone = InstalledFace { path: PathBuf::from("definitely/not/here.ttf"), ..inter.clone() };
        assert!(load(&gone).is_none());
    }

    #[test]
    fn classes_follow_the_name() {
        assert_eq!(class_of("courier new"), Class::Mono);
        assert_eq!(class_of("times new roman"), Class::Serif);
        assert_eq!(class_of("noto sans arabic"), Class::Sans);
        assert_eq!(class_of("noto serif"), Class::Serif);
        assert_eq!(class_of("din next lt arabic"), Class::Sans);
    }
}
