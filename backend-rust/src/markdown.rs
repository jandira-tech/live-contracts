#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkdownStatus { Done, Empty }

impl MarkdownStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            MarkdownStatus::Done => "done",
            MarkdownStatus::Empty => "empty",
        }
    }
}

/// Convert an HTML document to Markdown. Empty input or any conversion failure
/// yields "" (never panics). Output is trimmed. Mirrors converter.py.
pub fn html_to_markdown(html: &str) -> String {
    if html.trim().is_empty() {
        return String::new();
    }
    match htmd::convert(html) {
        Ok(md) => md.trim().to_string(),
        Err(_) => String::new(),
    }
}

/// Status for a produced markdown string. Conversion failures are swallowed to ""
/// upstream (html_to_markdown never panics), so the only outcomes are done/empty.
pub fn status_for(markdown: &str) -> MarkdownStatus {
    if markdown.trim().is_empty() { MarkdownStatus::Empty } else { MarkdownStatus::Done }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_yields_empty() {
        assert_eq!(html_to_markdown(""), "");
    }

    #[test]
    fn converts_basic_html() {
        let md = html_to_markdown("<h1>Title</h1><p>Hello <b>world</b></p>");
        assert!(md.contains("Title"));
        assert!(md.contains("Hello"));
        assert!(md.contains("world"));
        assert_eq!(md, md.trim()); // trimmed
    }

    #[test]
    fn status_from_markdown() {
        assert_eq!(status_for("# real content"), MarkdownStatus::Done);
        assert_eq!(status_for(""), MarkdownStatus::Empty);
        assert_eq!(status_for("   "), MarkdownStatus::Empty);
    }
}
