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
/// yields "" (never panics). Output is trimmed.
///
/// Uses quick_html2md with **tables flattened to prose**. SEC EX-10 HTML uses
/// `<table>` for LAYOUT (single-cell wrappers, empty spacer cells), so a
/// faithful converter (htmd, pandoc, …) emits empty-cell / empty-paragraph junk
/// that renders as dead whitespace. A 6-converter bake-off over 10 real filings
/// (.context/md-converters) showed this config alone yields 0 empty cells /
/// 0 empty paragraphs while keeping all text, emphasis, links and lists.
/// Falls back to htmd if quick_html2md yields nothing.
pub fn html_to_markdown(html: &str) -> String {
    if html.trim().is_empty() {
        return String::new();
    }
    let opts = quick_html2md::MarkdownOptions {
        preserve_headings: true,
        include_links: true,
        include_images: false, // relative SEC image URLs would render as broken refs
        preserve_emphasis: true,
        preserve_strikethrough: true,
        preserve_lists: true,
        preserve_code: true,
        preserve_blockquotes: true,
        preserve_tables: false, // SEC uses <table> for LAYOUT → flatten to prose
        max_heading_level: 6,
        commonmark: false,
        escape_special_chars: false,
        base_url: None,
    };
    let md = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        quick_html2md::html_to_markdown_with_options(html, &opts)
    }))
    .map(|s| s.trim().to_string())
    .unwrap_or_default();
    if !md.is_empty() {
        return md;
    }
    // Fallback: htmd (mature) if quick_html2md panicked or produced nothing.
    htmd::convert(html)
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
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
