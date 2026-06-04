/// True when `doc_type` is a traditional EX-10 material contract
/// (`EX-10` or `EX-10.N`), excluding XBRL `EX-101`/`EX-100` etc.
/// Mirrors backend/sec_listener/parsing.py::classify_documents.
pub fn is_traditional_ex10(doc_type: &str) -> bool {
    let t = doc_type.trim();
    if let Some(suffix) = t.strip_prefix("EX-10") {
        suffix.is_empty() || suffix.starts_with('.')
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_ex10_variants() {
        assert!(is_traditional_ex10("EX-10"));
        assert!(is_traditional_ex10("EX-10.1"));
        assert!(is_traditional_ex10("EX-10.27"));
        assert!(is_traditional_ex10("  EX-10.1  ")); // trimmed
        // XBRL and others are NOT traditional EX-10
        assert!(!is_traditional_ex10("EX-101"));
        assert!(!is_traditional_ex10("EX-101.INS"));
        assert!(!is_traditional_ex10("EX-100"));
        assert!(!is_traditional_ex10("EX-21"));
        assert!(!is_traditional_ex10("EX-99.1"));
        assert!(!is_traditional_ex10("10-K"));
        assert!(!is_traditional_ex10(""));
    }
}
