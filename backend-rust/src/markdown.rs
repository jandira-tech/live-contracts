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
/// Uses html2markdown (AST-to-AST), which keeps SEC EX-10 tables as GFM tables
/// rather than flattening them — chosen after a 6-converter bake-off over 10
/// real filings (.context/md-converters) for the most faithful rendering of the
/// original form/table layout. Falls back to htmd if html2markdown yields nothing.
pub fn html_to_markdown(html: &str) -> String {
    if html.trim().is_empty() {
        return String::new();
    }
    let md = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        html2markdown::convert(html)
    }))
    .map(|s| s.trim().to_string())
    .unwrap_or_default();
    if !md.is_empty() {
        return md;
    }
    // Fallback: htmd (mature) if html2markdown panicked or produced nothing.
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

    #[test]
    fn whitespace_only_input_yields_empty() {
        assert_eq!(html_to_markdown("   \n\t  "), "");
    }

    #[test]
    fn preserves_html_tables_as_gfm_tables() {
        // The whole point of switching to html2markdown: SEC EX-10 tables should
        // survive as GFM tables (pipe-delimited rows) rather than being flattened
        // into plain paragraphs.
        let html = "<table><tr><th>Name</th><th>Value</th></tr><tr><td>Rate</td><td>5%</td></tr></table>";
        let md = html_to_markdown(html);
        assert!(!md.is_empty());
        assert!(md.contains('|'), "expected a GFM table (pipe-delimited), got: {md}");
        assert!(md.contains("Name"));
        assert!(md.contains("Rate"));
        assert!(md.contains("5%"));
    }

    #[test]
    fn never_panics_on_malformed_or_unusual_html() {
        // html_to_markdown must swallow any panic from the underlying converter
        // (via catch_unwind) and fall back to htmd rather than crashing the
        // ingest pipeline on a single malformed filing.
        let inputs = [
            "<div><span>unclosed tags",
            "<table><tr><td>no closing table",
            "<!-- just a comment -->",
            "<p>&amp;&nbsp;&lt;weird entities&gt;</p>",
            "<script>alert('x')</script><p>after script</p>",
            "not html at all, just text",
            "<html><body><p>ok</p>",
        ];
        for html in inputs {
            // Should not panic; result may be empty or non-empty depending on the
            // converter, but must always be a valid (trimmed) String.
            let md = html_to_markdown(html);
            assert_eq!(md, md.trim());
        }
    }

    #[test]
    fn output_is_always_trimmed() {
        let md = html_to_markdown("   <p>  padded content  </p>   ");
        assert_eq!(md, md.trim());
        assert!(md.contains("padded content"));
    }

    #[test]
    fn converts_nested_formatting() {
        let md = html_to_markdown("<ul><li>First</li><li>Second <b>bold</b></li></ul>");
        assert!(md.contains("First"));
        assert!(md.contains("Second"));
        assert!(md.contains("bold"));
    }
}
