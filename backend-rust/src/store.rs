//! Optional local SQLite store for the stateful "Python-parity" mode.
//!
//! Enabled only when `SEC_STORE_PATH` is set; otherwise the producer stays
//! stateless (inline POST to /api/ingest). The schema mirrors the Python
//! listener's `ex10_listener.db` so the two are interchangeable.
//!
//! Connections are RAII (`rusqlite::Connection` closes on drop) and a single
//! pooled connection sits behind a `Mutex` — no per-call open/close churn, so
//! this can't reproduce the Python worker's fd leak.

use crate::ingest::IngestRecord;
use rusqlite::Connection;
use std::sync::Mutex;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS seen_accessions (
    accession TEXT PRIMARY KEY,
    first_seen TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    form_type TEXT,
    cik TEXT
);
CREATE TABLE IF NOT EXISTS ex10_exhibits (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    accession TEXT, cik TEXT, form_type TEXT, doc_type TEXT, filename TEXT,
    description TEXT, sequence TEXT, filing_url TEXT,
    found_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    markdown TEXT, markdown_status TEXT, filing_metadata TEXT, image_urls TEXT,
    mirrored INTEGER DEFAULT 0, image_attempts INTEGER DEFAULT 0,
    UNIQUE(accession, doc_type, filename)
);
CREATE TABLE IF NOT EXISTS all_exhibits (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    accession TEXT, cik TEXT, form_type TEXT, doc_type TEXT, filename TEXT,
    description TEXT, sequence TEXT, filing_url TEXT,
    found_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE IF NOT EXISTS rss_entries (
    accession TEXT, cik TEXT, form_type TEXT, filing_date TEXT,
    rss_summary TEXT, processed BOOLEAN DEFAULT 0,
    first_seen TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
"#;

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// Open a store at `path` (a file, or ":memory:" for tests). Sets WAL so the
    /// query API can read while the pipeline writes.
    pub fn open(path: &str) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(30))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Create the schema (idempotent).
    pub fn init(&self) -> rusqlite::Result<()> {
        self.conn.lock().unwrap().execute_batch(SCHEMA)
    }

    // The write paths below are exercised by tests now and wired into the
    // pipeline in the next PR (all-exhibit capture); allow until then.
    #[allow(dead_code)]
    pub fn is_seen(&self, accession: &str) -> rusqlite::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n: i64 = conn.query_row(
            "SELECT count(*) FROM seen_accessions WHERE accession = ?1",
            [accession],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    #[allow(dead_code)]
    pub fn mark_seen(&self, accession: &str, form_type: &str, cik: &str) -> rusqlite::Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT OR IGNORE INTO seen_accessions (accession, form_type, cik) VALUES (?1, ?2, ?3)",
            (accession, form_type, cik),
        )?;
        Ok(())
    }

    /// Insert (or update markdown/status/metadata/images on conflict) one EX-10 row.
    #[allow(dead_code)]
    pub fn upsert_ex10(&self, r: &IngestRecord) -> rusqlite::Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO ex10_exhibits
               (accession, cik, form_type, doc_type, filename, description, sequence,
                filing_url, found_at, markdown, markdown_status, filing_metadata, image_urls)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)
             ON CONFLICT(accession, doc_type, filename) DO UPDATE SET
               markdown         = excluded.markdown,
               markdown_status  = excluded.markdown_status,
               filing_metadata  = excluded.filing_metadata,
               image_urls       = excluded.image_urls",
            rusqlite::params![
                r.accession, r.cik, r.form_type, r.doc_type, r.filename, r.description,
                r.sequence, r.filing_url, r.found_at, r.markdown, r.markdown_status,
                r.filing_metadata, r.image_urls,
            ],
        )?;
        Ok(())
    }

    /// Record a non-EX-10 exhibit (metadata only) in `all_exhibits`.
    #[allow(dead_code)]
    pub fn insert_all_exhibit(&self, r: &IngestRecord) -> rusqlite::Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO all_exhibits
               (accession, cik, form_type, doc_type, filename, description, sequence, filing_url, found_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            rusqlite::params![
                r.accession, r.cik, r.form_type, r.doc_type, r.filename, r.description,
                r.sequence, r.filing_url, r.found_at,
            ],
        )?;
        Ok(())
    }

    pub fn count_ex10(&self) -> rusqlite::Result<i64> {
        self.conn
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM ex10_exhibits", [], |r| r.get(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(doc_type: &str, filename: &str) -> IngestRecord {
        IngestRecord {
            id: 1,
            accession: "0001-25-000001".into(),
            cik: "123".into(),
            form_type: "8-K".into(),
            doc_type: doc_type.into(),
            filename: filename.into(),
            description: "d".into(),
            sequence: "2".into(),
            filing_url: "https://sec.gov/x.txt".into(),
            found_at: "2025-02-01T00:00:00Z".into(),
            filed_at: "".into(),
            markdown_status: "done".into(),
            filing_metadata: None,
            image_urls: None,
            markdown: "# x".into(),
        }
    }

    fn mem() -> Store {
        let s = Store::open(":memory:").unwrap();
        s.init().unwrap();
        s
    }

    #[test]
    fn init_creates_all_four_tables() {
        let s = mem();
        let conn = s.conn.lock().unwrap();
        for t in ["seen_accessions", "ex10_exhibits", "all_exhibits", "rss_entries"] {
            let n: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [t],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 1, "missing table {t}");
        }
    }

    #[test]
    fn seen_roundtrip_is_idempotent() {
        let s = mem();
        assert!(!s.is_seen("A").unwrap());
        s.mark_seen("A", "8-K", "1").unwrap();
        assert!(s.is_seen("A").unwrap());
        s.mark_seen("A", "8-K", "1").unwrap(); // INSERT OR IGNORE — no error, still one
        assert!(s.is_seen("A").unwrap());
    }

    #[test]
    fn upsert_ex10_dedupes_on_accession_doctype_filename() {
        let s = mem();
        s.upsert_ex10(&rec("EX-10.1", "a.htm")).unwrap();
        let mut again = rec("EX-10.1", "a.htm");
        again.markdown_status = "empty".into();
        s.upsert_ex10(&again).unwrap(); // same key → update, not a new row
        assert_eq!(s.count_ex10().unwrap(), 1);
        s.upsert_ex10(&rec("EX-10.1", "b.htm")).unwrap(); // different filename → new row
        assert_eq!(s.count_ex10().unwrap(), 2);
    }

    #[test]
    fn all_exhibits_accepts_non_ex10() {
        let s = mem();
        s.insert_all_exhibit(&rec("EX-21", "subs.htm")).unwrap();
        let conn = s.conn.lock().unwrap();
        let n: i64 = conn
            .query_row("SELECT count(*) FROM all_exhibits", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }
}
