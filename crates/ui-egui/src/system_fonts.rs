//! The last-resort interface font: one face already installed on this machine.
//!
//! The embedded faces (Inter, egui's defaults, craft-fonts) come first in every family; this one
//! only draws characters none of them has, such as an Arabic file name in a build without
//! craft-fonts, or the Simplified Chinese interface in a build without a `Hans` face (#826). A
//! few more installed faces, [`devanagari_fallback`], [`telugu_fallback`], [`sinhala_fallback`]
//! and [`tamil_fallback`], draw Hindi, Telugu, Sinhala and Tamil when one is installed. It is
//! read at runtime and never embedded or shipped (AGENTS.md §1.4), and `PDFCRAFT_SYSTEM_FONTS=0`
//! turns it off (published screenshots do).

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use egui::FontData;

#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
mod fontconfig;

/// Larger files are not read: a font path is still untrusted input.
const MAX_BYTES: u64 = 32 << 20;
/// Chinese system faces are big collections (PingFang, STHeiti), so they get a higher cap.
const MAX_HAN_BYTES: u64 = 96 << 20;
/// Faces tried in a collection (`.ttc`).
const MAX_FACES: u32 = 16;
/// Arabic letter alef: the face must have it to be worth loading for Arabic script.
const PROBE: char = '\u{0627}';
/// Devanagari letter ka: the face must have it to cover Hindi.
const DEVANAGARI_PROBE: char = '\u{0915}';
/// Telugu letter ka: the face must have it to cover Telugu.
const TELUGU_PROBE: char = '\u{0C15}';
/// Sinhala letter ka (ක): the face must have it to cover Sinhala.
const SINHALA_PROBE: char = '\u{0D9A}';
/// Tamil letter ka (க): the face must have it to cover Tamil.
const TAMIL_PROBE: char = '\u{0B95}';
/// 欢 (U+6B22): Simplified only, absent from the Japanese craft-fonts faces.
const HANS_PROBE: char = '\u{6B22}';
/// 說 (U+8AAA): Traditional only, absent from the Japanese craft-fonts faces.
const HANT_PROBE: char = '\u{8AAA}';

/// What the installed face is wanted for, from the interface language.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Need {
    /// Scripts no embedded face has (Arabic in a build without craft-fonts).
    #[default]
    Other,
    /// Simplified Chinese interface text.
    Hans,
    /// Traditional Chinese interface text.
    Hant,
}

/// The installed fallback face for `need`, read once per need. `None` when it is turned off or no
/// candidate fits.
pub fn fallback(need: Need) -> Option<Arc<FontData>> {
    static OTHER: OnceLock<Option<Arc<FontData>>> = OnceLock::new();
    static HANS: OnceLock<Option<Arc<FontData>>> = OnceLock::new();
    static HANT: OnceLock<Option<Arc<FontData>>> = OnceLock::new();
    let cache = match need {
        Need::Other => &OTHER,
        Need::Hans => &HANS,
        Need::Hant => &HANT,
    };
    cache.get_or_init(|| load(need)).clone()
}

/// Whether installed faces may be read at all: `PDFCRAFT_SYSTEM_FONTS=0` turns them off.
fn enabled() -> bool {
    std::env::var_os("PDFCRAFT_SYSTEM_FONTS").is_none_or(|v| v != "0")
}

fn load(need: Need) -> Option<Arc<FontData>> {
    if !enabled() {
        return None;
    }
    let (probe, max) = match need {
        Need::Other => (PROBE, MAX_BYTES),
        Need::Hans => (HANS_PROBE, MAX_HAN_BYTES),
        Need::Hant => (HANT_PROBE, MAX_HAN_BYTES),
    };
    candidates(need).iter().find_map(|path| read(path, probe, max))
}

/// The installed fallback face for Devanagari (Hindi), read once. `None` when it is turned off
/// or no candidate is installed; the interface then draws what the embedded faces can.
pub fn devanagari_fallback() -> Option<Arc<FontData>> {
    static CACHE: OnceLock<Option<Arc<FontData>>> = OnceLock::new();
    CACHE.get_or_init(|| script_fallback(&devanagari_candidates(), DEVANAGARI_PROBE)).clone()
}

/// The installed fallback face for Telugu, read once. `None` when it is turned off or no
/// candidate is installed; the interface then draws what the embedded faces can.
pub fn telugu_fallback() -> Option<Arc<FontData>> {
    static CACHE: OnceLock<Option<Arc<FontData>>> = OnceLock::new();
    CACHE.get_or_init(|| script_fallback(&telugu_candidates(), TELUGU_PROBE)).clone()
}

/// The installed fallback face for Sinhala, read once. `None` when it is turned off or no
/// candidate is installed; the interface then draws what the embedded faces can.
pub fn sinhala_fallback() -> Option<Arc<FontData>> {
    static CACHE: OnceLock<Option<Arc<FontData>>> = OnceLock::new();
    CACHE.get_or_init(|| script_fallback(&sinhala_candidates(), SINHALA_PROBE)).clone()
}

/// The installed fallback face for Tamil, read once. `None` when it is turned off or no
/// candidate is installed; the interface then draws what the embedded faces can.
pub fn tamil_fallback() -> Option<Arc<FontData>> {
    static CACHE: OnceLock<Option<Arc<FontData>>> = OnceLock::new();
    CACHE.get_or_init(|| script_fallback(&tamil_candidates(), TAMIL_PROBE)).clone()
}

fn script_fallback(candidates: &[PathBuf], probe: char) -> Option<Arc<FontData>> {
    if !enabled() {
        return None;
    }
    candidates.iter().find_map(|path| read(path, probe, MAX_BYTES))
}

/// Well-known locations of Devanagari (Hindi) faces, best first.
fn devanagari_candidates() -> Vec<PathBuf> {
    if cfg!(windows) {
        ["Nirmala.ttf", "NirmalaB.ttf", "mangal.ttf", "aparaj.ttf", "utsah.ttf"].iter().map(|f| windows_fonts().join(f)).collect()
    } else if cfg!(target_os = "macos") {
        [
            "/System/Library/Fonts/Supplemental/DevanagariMT.ttc",
            "/System/Library/Fonts/Supplemental/DevanagariSangamMN.ttc",
            "/System/Library/Fonts/Supplemental/Kohinoor.ttc",
            "/System/Library/Fonts/Kohinoor.ttc",
            "/Library/Fonts/Arial Unicode.ttf",
        ]
        .iter()
        .map(PathBuf::from)
        .collect()
    } else {
        unix_fonts(&[
            "google-noto/NotoSansDevanagariUI-Regular.ttf",
            "google-noto/NotoSansDevanagari-Regular.ttf",
            "google-noto-vf/NotoSansDevanagari[wght].ttf",
            "truetype/noto/NotoSansDevanagari-Regular.ttf",
            "truetype/noto/NotoSansDevanagariUI-Regular.ttf",
            "noto/NotoSansDevanagari-Regular.ttf",
            "noto/NotoSansDevanagariUI-Regular.ttf",
            "lohit-devanagari-fonts/Lohit-Devanagari.ttf",
            "truetype/lohit-devanagari/Lohit-Devanagari.ttf",
            "truetype/deva/Lohit-Devanagari.ttf",
            "lohit-hindi/Lohit-Hindi.ttf",
            "google-droid-sans-fonts/DroidSansDevanagari-Regular.ttf",
            "samyak-fonts/Samyak-Devanagari.ttf",
            "gargi/Gargi.ttf",
        ])
    }
}

/// Well-known locations of Telugu faces, best first.
fn telugu_candidates() -> Vec<PathBuf> {
    if cfg!(windows) {
        ["Nirmala.ttc", "Nirmala.ttf", "gautami.ttf"].iter().map(|f| windows_fonts().join(f)).collect()
    } else if cfg!(target_os = "macos") {
        [
            "/System/Library/Fonts/KohinoorTelugu.ttc",
            "/System/Library/Fonts/Supplemental/Telugu Sangam MN.ttc",
            "/System/Library/Fonts/Supplemental/Telugu MN.ttc",
            "/Library/Fonts/Arial Unicode.ttf",
        ]
        .iter()
        .map(PathBuf::from)
        .collect()
    } else {
        unix_fonts(&[
            "google-noto/NotoSansTeluguUI-Regular.ttf",
            "google-noto/NotoSansTelugu-Regular.ttf",
            "google-noto-vf/NotoSansTelugu[wght].ttf",
            "google-noto-vf/NotoSansTelugu-UI[wght].ttf",
            "truetype/noto/NotoSansTelugu-Regular.ttf",
            "truetype/noto/NotoSansTeluguUI-Regular.ttf",
            "noto/NotoSansTelugu-Regular.ttf",
            "noto/NotoSansTeluguUI-Regular.ttf",
            "lohit-telugu-fonts/Lohit-Telugu.ttf",
            "truetype/lohit-telugu/Lohit-Telugu.ttf",
            "truetype/fonts-telu-extra/Pothana2000.ttf",
        ])
    }
}

/// Well-known locations of Sinhala faces, best first. Nirmala UI has Sinhala from Windows 10;
/// Iskoola Pota is the older Windows Sinhala face.
fn sinhala_candidates() -> Vec<PathBuf> {
    if cfg!(windows) {
        ["Nirmala.ttc", "Nirmala.ttf", "iskpota.ttf"].iter().map(|f| windows_fonts().join(f)).collect()
    } else if cfg!(target_os = "macos") {
        ["/System/Library/Fonts/Supplemental/Sinhala Sangam MN.ttc", "/System/Library/Fonts/Supplemental/Sinhala MN.ttc"]
            .iter()
            .map(PathBuf::from)
            .collect()
    } else {
        unix_fonts(&[
            "google-noto/NotoSansSinhalaUI-Regular.ttf",
            "google-noto/NotoSansSinhala-Regular.ttf",
            "google-noto-vf/NotoSansSinhala[wght].ttf",
            "truetype/noto/NotoSansSinhala-Regular.ttf",
            "truetype/noto/NotoSansSinhalaUI-Regular.ttf",
            "noto/NotoSansSinhala-Regular.ttf",
            "noto/NotoSansSinhalaUI-Regular.ttf",
        ])
    }
}

/// Well-known locations of Tamil faces, best first. Latha is the older Windows Tamil face.
fn tamil_candidates() -> Vec<PathBuf> {
    if cfg!(windows) {
        ["Nirmala.ttc", "Nirmala.ttf", "latha.ttf"].iter().map(|f| windows_fonts().join(f)).collect()
    } else if cfg!(target_os = "macos") {
        [
            "/System/Library/Fonts/Supplemental/Tamil Sangam MN.ttc",
            "/System/Library/Fonts/Supplemental/Tamil MN.ttc",
            "/Library/Fonts/Arial Unicode.ttf",
        ]
        .iter()
        .map(PathBuf::from)
        .collect()
    } else {
        unix_fonts(&[
            "google-noto/NotoSansTamilUI-Regular.ttf",
            "google-noto/NotoSansTamil-Regular.ttf",
            "google-noto-vf/NotoSansTamil[wght].ttf",
            "truetype/noto/NotoSansTamil-Regular.ttf",
            "truetype/noto/NotoSansTamilUI-Regular.ttf",
            "noto/NotoSansTamil-Regular.ttf",
            "noto/NotoSansTamilUI-Regular.ttf",
            "lohit-tamil-fonts/Lohit-Tamil.ttf",
            "truetype/lohit-tamil/Lohit-Tamil.ttf",
        ])
    }
}

/// Well-known locations of faces with broad script coverage, best first.
fn candidates(need: Need) -> Vec<PathBuf> {
    match need {
        Need::Other => other_candidates(),
        Need::Hans | Need::Hant => han_candidates(need == Need::Hant),
    }
}

fn other_candidates() -> Vec<PathBuf> {
    if cfg!(windows) {
        ["segoeui.ttf", "tahoma.ttf", "arial.ttf"].iter().map(|f| windows_fonts().join(f)).collect()
    } else if cfg!(target_os = "macos") {
        ["/System/Library/Fonts/SFArabic.ttf", "/System/Library/Fonts/GeezaPro.ttc", "/System/Library/Fonts/Supplemental/Arial.ttf"]
            .iter()
            .map(PathBuf::from)
            .collect()
    } else {
        unix_fonts(&[
            "truetype/noto/NotoSansArabic-Regular.ttf",
            "noto/NotoSansArabic-Regular.ttf",
            "google-noto/NotoSansArabic-Regular.ttf",
            "truetype/dejavu/DejaVuSans.ttf",
            "TTF/DejaVuSans.ttf",
            "dejavu/DejaVuSans.ttf",
            "dejavu-sans-fonts/DejaVuSans.ttf",
        ])
    }
}

/// Chinese sans faces the operating system ships, the UI language's own region first. Noto CJK
/// and Source Han are left out on purpose (AGENTS.md §1.1).
fn han_candidates(traditional: bool) -> Vec<PathBuf> {
    if cfg!(windows) {
        // Microsoft YaHei / JhengHei, then the older SimSun / MingLiU collections.
        let files: &[&str] =
            if traditional { &["msjh.ttc", "msyh.ttc", "mingliu.ttc", "simsun.ttc"] } else { &["msyh.ttc", "msjh.ttc", "simsun.ttc", "mingliu.ttc"] };
        files.iter().map(|f| windows_fonts().join(f)).collect()
    } else if cfg!(target_os = "macos") {
        let files: &[&str] = if traditional {
            &["PingFang.ttc", "STHeiti Light.ttc", "STHeiti Medium.ttc", "Hiragino Sans GB.ttc"]
        } else {
            &["PingFang.ttc", "Hiragino Sans GB.ttc", "STHeiti Light.ttc", "STHeiti Medium.ttc"]
        };
        files.iter().map(|f| Path::new("/System/Library/Fonts").join(f)).collect()
    } else {
        unix_fonts(&[
            "truetype/wqy/wqy-microhei.ttc",
            "wenquanyi/wqy-microhei/wqy-microhei.ttc",
            "wqy-microhei/wqy-microhei.ttc",
            "truetype/wqy/wqy-zenhei.ttc",
            "wenquanyi/wqy-zenhei/wqy-zenhei.ttc",
            "wqy-zenhei/wqy-zenhei.ttc",
            "truetype/droid/DroidSansFallbackFull.ttf",
            "google-droid-sans-fonts/DroidSansFallbackFull.ttf",
            "droid/DroidSansFallbackFull.ttf",
            "truetype/arphic/uming.ttc",
            "arphic-uming/uming.ttc",
        ])
    }
}

fn windows_fonts() -> PathBuf {
    let dir = std::env::var_os("WINDIR").or_else(|| std::env::var_os("SystemRoot")).map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    dir.join("Fonts")
}

fn unix_fonts(files: &'static [&'static str]) -> Vec<PathBuf> {
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    static INSTALLED: OnceLock<Vec<PathBuf>> = OnceLock::new();
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    // Read once per process, and not at all when installed fonts are turned off.
    let installed = INSTALLED
        .get_or_init(|| {
            if !enabled() {
                return Vec::new();
            }
            fontconfig::font_files(&fontconfig::config_path())
        })
        .as_slice();
    #[cfg(not(all(unix, not(target_os = "macos"), not(target_arch = "wasm32"))))]
    let installed: &[PathBuf] = &[];
    unix_candidates(files, installed)
}

fn unix_candidates(files: &[&str], installed: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths: Vec<_> =
        ["/usr/share/fonts", "/usr/local/share/fonts"].iter().flat_map(|dir| files.iter().map(move |f| Path::new(dir).join(f))).collect();
    // Fontconfig may point into /nix/store or a user's font directory. Keep the existing
    // candidate names and their preference order; this does not select arbitrary fonts.
    for file in files {
        let name = Path::new(file).file_name();
        for path in installed.iter().filter(|p| p.file_name() == name) {
            if !paths.contains(path) {
                paths.push(path.clone());
            }
        }
    }
    paths
}

/// Faces already read, by file and face index. One installed file often serves several scripts
/// (Nirmala UI has Devanagari, Telugu, Sinhala and Tamil; Arial Unicode on macOS three of them),
/// and without this each script would read and keep its own copy of the whole file.
static LOADED: Mutex<Vec<(PathBuf, u32, Arc<FontData>)>> = Mutex::new(Vec::new());

fn read(path: &Path, probe: char, max: u64) -> Option<Arc<FontData>> {
    // A poisoned lock only means another thread panicked mid-push; the list is still valid.
    let mut loaded = LOADED.lock().unwrap_or_else(PoisonError::into_inner);
    read_cached(&mut loaded, path, probe, max)
}

fn read_cached(loaded: &mut Vec<(PathBuf, u32, Arc<FontData>)>, path: &Path, probe: char, max: u64) -> Option<Arc<FontData>> {
    if let Some((_, _, data)) = loaded
        .iter()
        .find(|(p, index, data)| p == path && u64::try_from(data.font.len()).is_ok_and(|len| len <= max) && maps(&data.font, *index, probe))
    {
        return Some(Arc::clone(data));
    }
    let bytes = read_capped(path, max)?;
    let index = face_with(&bytes, probe)?;
    let mut data = FontData::from_owned(bytes);
    data.index = index;
    log::info!("interface font fallback: {} (face {index})", path.display());
    let data = Arc::new(crate::theme::on_interface_baseline(data));
    loaded.push((path.to_owned(), index, Arc::clone(&data)));
    Some(data)
}

/// The file's bytes, or `None` when it is not a regular file or is larger than `max`. The read
/// itself is capped too, so a file that grows after the size check can't make it unbounded.
fn read_capped(path: &Path, max: u64) -> Option<Vec<u8>> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > max {
        return None;
    }
    let mut bytes = Vec::with_capacity(usize::try_from(meta.len()).ok()?);
    std::fs::File::open(path).ok()?.take(max.saturating_add(1)).read_to_end(&mut bytes).ok()?;
    (u64::try_from(bytes.len()).ok()? <= max).then_some(bytes)
}

/// Whether face `index` of the file parses and maps `c`.
fn maps(bytes: &[u8], index: u32, c: char) -> bool {
    use skrifa::MetadataProvider as _;
    skrifa::FontRef::from_index(bytes, index).is_ok_and(|font| font.charmap().map(c).is_some())
}

/// The first face of the file that parses and maps `c`. egui parses fonts with the same skrifa,
/// so a face accepted here is one it can load.
fn face_with(bytes: &[u8], c: char) -> Option<u32> {
    (0..MAX_FACES).find(|&index| maps(bytes, index, c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_candidates_preserve_names_order_and_existing_paths() {
        let names = ["noto/NotoSansArabic-Regular.ttf", "truetype/dejavu/DejaVuSans.ttf"];
        let noto = PathBuf::from("/custom/fonts/NotoSansArabic-Regular.ttf");
        let dejavu = PathBuf::from("/nix/store/font/share/fonts/truetype/DejaVuSans.ttf");
        let installed = [dejavu.clone(), PathBuf::from("/custom/fonts/SourceHanSans.ttf"), noto.clone(), dejavu.clone()];
        let mut expected = unix_candidates(&names, &[]);
        expected.extend([noto, dejavu]);
        assert_eq!(unix_candidates(&names, &installed), expected);
    }

    #[test]
    fn broken_and_missing_files_are_skipped() {
        assert_eq!(face_with(b"", PROBE), None);
        assert_eq!(face_with(b"not a font at all", PROBE), None);
        assert_eq!(face_with(&[0u8; 4096], PROBE), None);
        assert!(read(Path::new("definitely/not/here.ttf"), PROBE, MAX_BYTES).is_none());
        // A directory is not a font.
        assert!(read(&std::env::temp_dir(), PROBE, MAX_BYTES).is_none());
    }

    #[test]
    fn a_face_without_the_probe_is_rejected() {
        // Inter is Latin, Greek and Cyrillic only.
        let inter = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
        assert_eq!(face_with(inter, PROBE), None);
        assert_eq!(face_with(inter, HANS_PROBE), None);
        assert_eq!(face_with(inter, HANT_PROBE), None);
        assert_eq!(face_with(inter, 'A'), Some(0));
    }

    #[test]
    fn a_file_over_the_cap_is_not_read() {
        let inter = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/Inter-Regular.ttf"));
        assert!(read(inter, 'A', MAX_BYTES).is_some());
        assert!(read(inter, 'A', 1024).is_none());
    }

    #[test]
    fn one_file_serving_several_scripts_is_read_once() {
        let inter = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/Inter-Regular.ttf"));
        let mut loaded = Vec::new();
        let latin = read_cached(&mut loaded, inter, 'A', MAX_BYTES).expect("Inter has A");
        // Cyrillic from the same face: the same bytes, not a second copy.
        let cyrillic = read_cached(&mut loaded, inter, '\u{0416}', MAX_BYTES).expect("Inter has Ж");
        assert!(Arc::ptr_eq(&latin, &cyrillic));
        assert_eq!(loaded.len(), 1);
        // A probe the face lacks is still rejected, cached or not.
        assert!(read_cached(&mut loaded, inter, PROBE, MAX_BYTES).is_none());
        assert_eq!(loaded.len(), 1);
    }

    #[test]
    fn the_read_is_capped_at_the_limit() {
        let inter = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/Inter-Regular.ttf"));
        let len = std::fs::metadata(inter).expect("asset").len();
        assert_eq!(read_capped(inter, len).map(|b| b.len() as u64), Some(len));
        assert!(read_capped(inter, len - 1).is_none());
        assert!(read_capped(Path::new("definitely/not/here.ttf"), MAX_BYTES).is_none());
    }

    #[test]
    fn candidates_are_absolute_font_files() {
        for need in [Need::Other, Need::Hans, Need::Hant] {
            let list = candidates(need);
            assert!(!list.is_empty(), "{need:?}");
            assert!(list.iter().all(|p| p.extension().is_some_and(|e| e == "ttf" || e == "ttc")), "{need:?}");
        }
    }

    #[test]
    fn a_script_fallback_is_the_first_candidate_with_the_probe() {
        let inter = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/Inter-Regular.ttf"));
        let candidates = [PathBuf::from("definitely/not/here.ttf"), inter];
        assert!(script_fallback(&candidates, 'A').is_some());
        assert!(script_fallback(&candidates, TELUGU_PROBE).is_none());
        assert!(script_fallback(&candidates, DEVANAGARI_PROBE).is_none());
        assert!(script_fallback(&candidates, SINHALA_PROBE).is_none());
        assert!(script_fallback(&candidates, TAMIL_PROBE).is_none());
    }

    #[test]
    fn script_candidates_are_absolute_font_files() {
        for list in [devanagari_candidates(), telugu_candidates(), sinhala_candidates(), tamil_candidates()] {
            assert!(!list.is_empty());
            assert!(list.iter().all(|p| p.is_absolute() || cfg!(windows)), "{list:?}");
            assert!(list.iter().all(|p| p.extension().is_some_and(|e| e == "ttf" || e == "ttc")), "{list:?}");
        }
    }

    #[test]
    fn han_candidates_skip_adobe_designs() {
        for need in [Need::Hans, Need::Hant] {
            for path in candidates(need) {
                let name = path.to_string_lossy().to_lowercase();
                assert!(!["notosanscjk", "notoserifcjk", "sourcehan", "source-han"].iter().any(|b| name.contains(b)), "{name}");
            }
        }
    }
}
