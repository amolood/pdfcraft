//! Explicit document-font discovery over the installed fonts (fork: system fonts included).
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct FontFile {
    pub path: PathBuf,
    pub family: String,
    pub face: String,
}

/// The system and user font folders of this platform; discovery is performed only on request.
pub fn installed_directories() -> Vec<PathBuf> {
    pdfcraft_fonts::installed::directories()
}

/// Bounded, symlink-free scan of standalone, static faces whose `fsType` allows editable
/// embedding (the same index the automatic fallback of Edit text uses). The caller controls
/// roots (automation resolves confinement before calling this).
pub fn discover(roots: &[PathBuf]) -> Vec<FontFile> {
    let mut files: Vec<FontFile> =
        pdfcraft_fonts::installed::scan(roots).into_iter().map(|f| FontFile { path: f.path, family: f.family, face: f.postscript }).collect();
    files.sort_by(|a, b| a.face.cmp(&b.face).then(a.path.cmp(&b.path)));
    files
}
