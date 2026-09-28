mod detail_dialog;
mod glyph_page;
mod history_row;

pub use detail_dialog::DetailDialog;
pub use glyph_page::GlyphPage;
pub use history_row::HistoryRow;
pub(crate) use history_row::relative_time;

/// Cheap heuristic for showing text in a monospace font.
pub fn looks_like_code(text: &str) -> bool {
    let sample: String = text.chars().take(2000).collect();
    let braces = sample.matches(['{', '}', ';']).count();
    let indented = sample
        .lines()
        .filter(|l| l.starts_with("    ") || l.starts_with('\t'))
        .count();
    let tags = sample.matches("</").count();
    sample.starts_with("#!")
        || sample.starts_with("$ ")
        || braces >= 3
        || tags >= 2
        || (indented >= 2 && sample.lines().count() >= 3)
}
