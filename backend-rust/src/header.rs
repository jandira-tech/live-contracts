use serde::Serialize;
use std::collections::BTreeMap;

/// A flattened key/value from secinfra's standardized submission metadata.
/// `key` is the leaf key (e.g. "conformed-name"); `value` the decoded text.
pub struct Event {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Serialize, Default)]
pub struct FilingHeader {
    pub company_name: String,
    pub cik: String,
    pub sic: String,
    pub state_of_incorporation: String,
    pub period: String,
    pub filing_date: String,
    pub filed_at: String,
    pub file_number: String,
    pub location: String,
    pub items: Vec<String>,
}

fn first<'a>(map: &'a BTreeMap<String, Vec<String>>, key: &str) -> &'a str {
    map.get(key).and_then(|v| v.first()).map(|s| s.as_str()).unwrap_or("")
}

/// Build the compact filing header from standardized metadata events.
/// Never panics; missing fields default to empty. Mirrors
/// parsing.py::extract_filing_header.
pub fn build_header(events: &[Event]) -> FilingHeader {
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for e in events {
        let k = e.key.trim().to_lowercase();
        if k.is_empty() { continue; }
        map.entry(k).or_default().push(e.value.trim().to_string());
    }
    let city = first(&map, "city");
    let state = first(&map, "state");
    let location = [city, state]
        .iter()
        .filter(|p| !p.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    FilingHeader {
        company_name: first(&map, "conformed-name").to_string(),
        cik: first(&map, "cik").to_string(),
        sic: first(&map, "assigned-sic").to_string(),
        state_of_incorporation: first(&map, "state-of-incorporation").to_string(),
        period: first(&map, "period").to_string(),
        filing_date: first(&map, "filing-date").to_string(),
        filed_at: first(&map, "acceptance-datetime").to_string(),
        file_number: first(&map, "file-number").to_string(),
        location,
        items: map.get("item-information").cloned().unwrap_or_default(),
    }
}

/// Serialize the header to the JSON string stored in `filing_metadata`.
pub fn header_json(events: &[Event]) -> String {
    serde_json::to_string(&build_header(events)).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kv(key: &str, val: &str) -> Event {
        Event { key: key.to_string(), value: val.to_string() }
    }

    #[test]
    fn builds_full_header_with_defaults() {
        let events = vec![]; // nothing → all defaults
        let h = build_header(&events);
        assert_eq!(h.company_name, "");
        assert_eq!(h.location, "");
        assert!(h.items.is_empty());
        // serializes with all keys present
        let v: serde_json::Value = serde_json::from_str(&header_json(&events)).unwrap();
        for k in ["company_name","cik","sic","state_of_incorporation","period",
                  "filing_date","filed_at","file_number","location","items"] {
            assert!(v.get(k).is_some(), "missing key {k}");
        }
    }

    #[test]
    fn selects_fields_and_joins_location() {
        let events = vec![
            kv("conformed-name", "ACME CORP"),
            kv("cik", "0000123"),
            kv("assigned-sic", "7372"),
            kv("state-of-incorporation", "DE"),
            kv("file-number", "001-12345"),
            kv("period", "20250131"),
            kv("filing-date", "20250201"),
            kv("acceptance-datetime", "20250201080000"),
            kv("city", "New York"),
            kv("state", "NY"),
            kv("item-information", "Entry into a Material Definitive Agreement"),
        ];
        let h = build_header(&events);
        assert_eq!(h.company_name, "ACME CORP");
        assert_eq!(h.cik, "0000123");
        assert_eq!(h.sic, "7372");
        assert_eq!(h.state_of_incorporation, "DE");
        assert_eq!(h.file_number, "001-12345");
        assert_eq!(h.period, "20250131");
        assert_eq!(h.filing_date, "20250201");
        assert_eq!(h.filed_at, "20250201080000");
        assert_eq!(h.location, "New York, NY");
        assert_eq!(h.items, vec!["Entry into a Material Definitive Agreement"]);
    }

    #[test]
    fn location_skips_blank_parts() {
        let events = vec![kv("city", "Boston")]; // no state
        assert_eq!(build_header(&events).location, "Boston");
        let events = vec![kv("state", "MA")]; // no city
        assert_eq!(build_header(&events).location, "MA");
    }
}
