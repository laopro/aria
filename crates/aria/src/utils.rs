use directories::ProjectDirs;
use unicode_bidi::{BidiInfo, Level};

/// Get directories for the project for config, cache, etc.
pub fn get_project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("com", "laopro", "aria")
}

/// Reorder a string using the Unicode Bidirectional Algorithm.
/// This ensures RTL text (e.g. Hebrew, Arabic) displays correctly in the terminal.
pub fn to_bidi_string(s: &str) -> String {
    let bidi_info = BidiInfo::new(s, None);
    if let Some(para) = bidi_info.paragraphs.first() {
        if para.level != Level::ltr() {
            return bidi_info.reorder_line(para, para.range.clone()).to_string();
        }
        // Check if any character has RTL level even in an LTR paragraph
        let start = para.range.start;
        let end = para.range.end;
        if bidi_info.levels[start..end]
            .iter()
            .any(|l| *l != Level::ltr())
        {
            return bidi_info.reorder_line(para, para.range.clone()).to_string();
        }
    }
    s.to_string()
}
