//! Durable outbox for rows the ingest route did not accept. Before this, a failed
//! POST was logged and the batch was gone; now it waits here and the drain loop
//! retries it. Keyed on the same (accession, doc_type, filename) triple as D1, so a
//! row that fails twice is stored once.
use crate::ingest::IngestRecord;
use rusqlite::Connection;
use std::sync::Mutex;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS outbox (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    accession TEXT NOT NULL,
    doc_type TEXT NOT NULL,
    filename TEXT NOT NULL,
    payload TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    queued_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (accession, doc_type, filename)
);
"#;

pub struct Outbox {
    conn: Mutex<Connection>,
}

impl Outbox {
    /// Open (and create) the outbox at `path`, or ":memory:" for tests.
    pub fn open(path: &str) -> rusqlite::Result<Self> {
        if path != ":memory:"
            && let Some(parent) = std::path::Path::new(path).parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|e| {
                rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CANTOPEN),
                    Some(format!("create parent dir {}: {e}", parent.display())),
                )
            })?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(30))?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    /// Queue rows; a row already queued is replaced by the newer copy.
    pub fn put(&self, rows: &[IngestRecord], reason: &str) -> rusqlite::Result<usize> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO outbox (accession, doc_type, filename, payload, last_error)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (accession, doc_type, filename)
                 DO UPDATE SET payload = excluded.payload, last_error = excluded.last_error",
            )?;
            for r in rows {
                let payload = serde_json::to_string(r)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                stmt.execute(rusqlite::params![r.accession, r.doc_type, r.filename, payload, reason])?;
            }
        }
        tx.commit()?;
        Ok(rows.len())
    }

    /// Oldest queued rows first, at most `max`.
    pub fn peek(&self, max: usize) -> rusqlite::Result<Vec<(i64, IngestRecord)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id, payload FROM outbox ORDER BY id LIMIT ?1")?;
        let rows = stmt.query_map([max as i64], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (id, payload) = row?;
            match serde_json::from_str::<IngestRecord>(&payload) {
                Ok(rec) => out.push((id, rec)),
                // A row this build cannot read is left in place, not deleted.
                Err(e) => tracing::error!(outbox_id = id, "unreadable outbox row: {e}"),
            }
        }
        Ok(out)
    }

    /// Remove delivered rows.
    pub fn ack(&self, ids: &[i64]) -> rusqlite::Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        for id in ids {
            tx.execute("DELETE FROM outbox WHERE id = ?1", [id])?;
        }
        tx.commit()
    }

    /// Record a failed delivery attempt; the rows stay queued.
    pub fn note_failure(&self, ids: &[i64], reason: &str) -> rusqlite::Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        for id in ids {
            tx.execute(
                "UPDATE outbox SET attempts = attempts + 1, last_error = ?2 WHERE id = ?1",
                rusqlite::params![id, reason],
            )?;
        }
        tx.commit()
    }

    pub fn len(&self) -> rusqlite::Result<i64> {
        self.conn.lock().unwrap().query_row("SELECT count(*) FROM outbox", [], |r| r.get(0))
    }
}

#[cfg(test)]
pub fn tests_rec(accession: &str, filename: &str) -> IngestRecord {
    tests::rec(accession, filename)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn rec(accession: &str, filename: &str) -> IngestRecord {
        IngestRecord {
            id: 7,
            accession: accession.into(),
            cik: "123".into(),
            form_type: "8-K".into(),
            doc_type: "EX-10.1".into(),
            filename: filename.into(),
            description: "Material Contract".into(),
            sequence: "2".into(),
            filing_url: "https://www.sec.gov/x.txt".into(),
            found_at: "2026-10-02T08:00:00Z".into(),
            filed_at: "20261002080000".into(),
            markdown_status: "done".into(),
            filing_metadata: None,
            image_urls: None,
            markdown: "# body".into(),
            source: "efts".into(),
            size_bytes: Some(10),
            detected_at: "2026-10-02T08:00:01+00:00".into(),
        }
    }

    fn mem() -> Outbox {
        Outbox::open(":memory:").unwrap()
    }

    #[test]
    fn put_then_peek_round_trips_rows_in_order() {
        let o = mem();
        assert_eq!(o.len().unwrap(), 0);
        o.put(&[rec("A", "a.htm"), rec("B", "b.htm")], "status 500").unwrap();
        assert_eq!(o.len().unwrap(), 2);
        let got = o.peek(10).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].1.accession, "A");
        assert_eq!(got[1].1.accession, "B");
        assert_eq!(got[0].1.markdown, "# body");
        assert_eq!(got[0].1.size_bytes, Some(10));
        assert_eq!(o.peek(1).unwrap().len(), 1);
    }

    #[test]
    fn same_row_queued_twice_is_stored_once() {
        let o = mem();
        o.put(&[rec("A", "a.htm")], "first").unwrap();
        let mut newer = rec("A", "a.htm");
        newer.markdown = "# newer".into();
        o.put(&[newer], "second").unwrap();
        assert_eq!(o.len().unwrap(), 1);
        assert_eq!(o.peek(1).unwrap()[0].1.markdown, "# newer");
    }

    #[test]
    fn ack_removes_and_note_failure_keeps() {
        let o = mem();
        o.put(&[rec("A", "a.htm"), rec("B", "b.htm")], "x").unwrap();
        let ids: Vec<i64> = o.peek(10).unwrap().into_iter().map(|(id, _)| id).collect();
        o.note_failure(&ids[1..], "status 401").unwrap();
        o.ack(&ids[..1]).unwrap();
        let left = o.peek(10).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].1.accession, "B");
        let conn = o.conn.lock().unwrap();
        let (attempts, err): (i64, String) = conn
            .query_row("SELECT attempts, last_error FROM outbox", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!(attempts, 1);
        assert_eq!(err, "status 401");
    }

    #[test]
    fn file_backed_outbox_survives_reopen() {
        let dir = std::env::temp_dir().join(format!("outbox-test-{}", std::process::id()));
        let path = dir.join("nested").join("outbox.db");
        let p = path.to_str().unwrap();
        Outbox::open(p).unwrap().put(&[rec("A", "a.htm")], "x").unwrap();
        assert_eq!(Outbox::open(p).unwrap().len().unwrap(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }
}
