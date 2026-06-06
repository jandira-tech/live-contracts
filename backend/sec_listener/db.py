"""SQLite data-access layer for the SEC EX-10 listener.

Centralises schema creation, idempotent migrations (notably the ``markdown``
column added in v0.2), and the queries the listener, worker and API share.
All connections use WAL mode so the listener can write while the API reads.
"""
from __future__ import annotations

import json
import sqlite3
from typing import Any

EX10_COLUMNS = (
    "accession",
    "cik",
    "form_type",
    "doc_type",
    "filename",
    "description",
    "sequence",
    "filing_url",
)


class Database:
    def __init__(self, path: str = "ex10_listener.db"):
        self.path = path

    # --- connection helpers -------------------------------------------------
    def connect(self) -> sqlite3.Connection:
        conn = sqlite3.connect(self.path, timeout=30)
        conn.row_factory = sqlite3.Row
        conn.execute("PRAGMA journal_mode=WAL")
        conn.execute("PRAGMA busy_timeout=30000")
        return conn

    def column_names(self, table: str) -> list[str]:
        with self.connect() as conn:
            return [r[1] for r in conn.execute(f"PRAGMA table_info({table})").fetchall()]

    # --- schema -------------------------------------------------------------
    def init(self) -> None:
        with self.connect() as conn:
            conn.executescript(
                """
                CREATE TABLE IF NOT EXISTS seen_accessions (
                    accession TEXT PRIMARY KEY,
                    first_seen TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                    form_type TEXT,
                    cik TEXT
                );
                CREATE TABLE IF NOT EXISTS ex10_exhibits (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    accession TEXT,
                    cik TEXT,
                    form_type TEXT,
                    doc_type TEXT,
                    filename TEXT,
                    description TEXT,
                    sequence TEXT,
                    filing_url TEXT,
                    found_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                    source TEXT,
                    size_bytes INTEGER,
                    detected_at TEXT,
                    UNIQUE(accession, doc_type, filename)
                );
                CREATE TABLE IF NOT EXISTS all_exhibits (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    accession TEXT, cik TEXT, form_type TEXT, doc_type TEXT,
                    filename TEXT, description TEXT, sequence TEXT, filing_url TEXT,
                    found_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
                );
                CREATE TABLE IF NOT EXISTS rss_entries (
                    accession TEXT PRIMARY KEY,
                    cik TEXT, form_type TEXT, filing_date TEXT, rss_summary TEXT,
                    processed BOOLEAN DEFAULT FALSE,
                    first_seen TIMESTAMP DEFAULT CURRENT_TIMESTAMP
                );
                """
            )
            self._ensure_column(conn, "ex10_exhibits", "markdown", "TEXT")
            self._ensure_column(conn, "ex10_exhibits", "markdown_status", "TEXT")
            self._ensure_column(conn, "ex10_exhibits", "filing_metadata", "TEXT")
            self._ensure_column(conn, "ex10_exhibits", "image_urls", "TEXT")
            self._ensure_column(conn, "ex10_exhibits", "mirrored", "INTEGER DEFAULT 0")
            self._ensure_column(conn, "ex10_exhibits", "image_attempts", "INTEGER DEFAULT 0")
            # Producer-parity columns (Rust producer + D1 schema): discovery channel,
            # submission size, and the precise RFC3339 detection timestamp. Nullable.
            self._ensure_column(conn, "ex10_exhibits", "source", "TEXT")
            self._ensure_column(conn, "ex10_exhibits", "size_bytes", "INTEGER")
            self._ensure_column(conn, "ex10_exhibits", "detected_at", "TEXT")
            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_ex10_found_at ON ex10_exhibits(found_at)"
            )
            # Expression index backing the filed_at ORDER BY (avoids a JSON parse +
            # filesort per row as the table grows).
            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_ex10_filed_at "
                "ON ex10_exhibits(json_extract(filing_metadata, '$.filed_at'))"
            )
            conn.commit()

    @staticmethod
    def _ensure_column(conn: sqlite3.Connection, table: str, column: str, decl: str) -> None:
        existing = [r[1] for r in conn.execute(f"PRAGMA table_info({table})").fetchall()]
        if column not in existing:
            conn.execute(f"ALTER TABLE {table} ADD COLUMN {column} {decl}")

    # --- writes -------------------------------------------------------------
    def save_ex10_exhibit(
        self,
        exhibit: dict[str, Any],
        markdown: str | None = None,
        filing_metadata: dict | None = None,
    ) -> None:
        status = "done" if markdown else "pending"
        # Distinguish "no metadata supplied" (None -> NULL, still pending backfill)
        # from "processed, but headerless" ({} -> '{}', a terminal state). Using a
        # falsy check would collapse {} to NULL and re-queue it forever.
        meta_json = json.dumps(filing_metadata) if filing_metadata is not None else None
        # found_at: the producer supplies the UTC display timestamp; fall back to the
        # SQLite CURRENT_TIMESTAMP default (also UTC) when absent (e.g. legacy callers).
        found_at = exhibit.get("found_at")
        with self.connect() as conn:
            conn.execute(
                """
                INSERT OR IGNORE INTO ex10_exhibits
                (accession, cik, form_type, doc_type, filename, description,
                 sequence, filing_url, markdown, markdown_status, filing_metadata,
                 source, size_bytes, detected_at, found_at)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
                        COALESCE(?, CURRENT_TIMESTAMP))
                """,
                (
                    exhibit.get("accession"),
                    exhibit.get("cik"),
                    exhibit.get("form_type"),
                    exhibit.get("doc_type"),
                    exhibit.get("filename"),
                    exhibit.get("description"),
                    exhibit.get("sequence"),
                    exhibit.get("url") or exhibit.get("filing_url"),
                    markdown,
                    status,
                    meta_json,
                    exhibit.get("source"),
                    exhibit.get("size_bytes"),
                    exhibit.get("detected_at"),
                    found_at,
                ),
            )
            conn.commit()

    def save_all_exhibit(self, exhibit: dict[str, Any]) -> None:
        with self.connect() as conn:
            conn.execute(
                """
                INSERT INTO all_exhibits
                (accession, cik, form_type, doc_type, filename, description, sequence, filing_url)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    exhibit.get("accession"),
                    exhibit.get("cik"),
                    exhibit.get("form_type"),
                    exhibit.get("doc_type"),
                    exhibit.get("filename"),
                    exhibit.get("description"),
                    exhibit.get("sequence"),
                    exhibit.get("url") or exhibit.get("filing_url"),
                ),
            )
            conn.commit()

    def save_rss_entry(self, accession, cik, form_type, filing_date, summary) -> None:
        with self.connect() as conn:
            conn.execute(
                """INSERT OR IGNORE INTO rss_entries
                   (accession, cik, form_type, filing_date, rss_summary)
                   VALUES (?, ?, ?, ?, ?)""",
                (accession, cik, form_type, filing_date, summary),
            )
            conn.commit()

    def is_accession_seen(self, accession: str) -> bool:
        with self.connect() as conn:
            return (
                conn.execute(
                    "SELECT 1 FROM seen_accessions WHERE accession = ?", (accession,)
                ).fetchone()
                is not None
            )

    def mark_accession_seen(self, accession, form_type, cik) -> None:
        with self.connect() as conn:
            conn.execute(
                "INSERT OR IGNORE INTO seen_accessions (accession, form_type, cik) VALUES (?, ?, ?)",
                (accession, form_type, cik),
            )
            conn.commit()

    def update_markdown(self, exhibit_id: int, markdown: str, status: str = "done") -> None:
        with self.connect() as conn:
            conn.execute(
                "UPDATE ex10_exhibits SET markdown = ?, markdown_status = ? WHERE id = ?",
                (markdown, status, exhibit_id),
            )
            conn.commit()

    def update_filing_metadata(self, exhibit_id: int, filing_metadata: dict) -> None:
        with self.connect() as conn:
            conn.execute(
                "UPDATE ex10_exhibits SET filing_metadata = ? WHERE id = ?",
                (json.dumps(filing_metadata), exhibit_id),
            )
            conn.commit()

    def update_image_urls(self, exhibit_id: int, urls: list[str]) -> None:
        # Reset mirrored so a row that was already pushed to D1 on markdown+metadata
        # (image capture is decoupled from the push) is re-queued for a follow-up push,
        # propagating the freshly captured image_urls via the ingest upsert. A row not
        # yet mirrored is unaffected (mirrored stays 0).
        with self.connect() as conn:
            conn.execute(
                "UPDATE ex10_exhibits SET image_urls = ?, mirrored = 0 WHERE id = ?",
                (json.dumps(urls), exhibit_id),
            )
            conn.commit()

    def exhibits_pending_images(self, limit: int = 25) -> list[dict[str, Any]]:
        """Converted exhibits not yet checked for images that reference an image file.

        image_urls IS NULL = not yet processed; once checked it's '[]' or a URL list.
        Returns full markdown (image-only bodies are short) so the caller can confirm
        with is_image_only().
        """
        with self.connect() as conn:
            rows = conn.execute(
                """SELECT id, accession, cik, markdown FROM ex10_exhibits
                   WHERE image_urls IS NULL AND markdown_status = 'done'
                     AND (markdown LIKE '%.jpg%' OR markdown LIKE '%.jpeg%'
                          OR markdown LIKE '%.png%' OR markdown LIKE '%.gif%'
                          OR markdown LIKE '%.tif%' OR markdown LIKE '%.tiff%'
                          OR markdown LIKE '%.svg%' OR markdown LIKE '%.webp%')
                   ORDER BY found_at DESC, id DESC LIMIT ?""",
                (limit,),
            ).fetchall()
        return [dict(r) for r in rows]

    def insert_exhibits_bulk(self, records: list[dict[str, Any]]) -> int:
        """Load exhibit rows (e.g. restored from the HF dataset) preserving found_at.

        INSERT OR IGNORE on UNIQUE(accession, doc_type, filename) so it merges with
        existing rows idempotently. Returns the number actually inserted.
        """
        inserted = 0
        with self.connect() as conn:
            for r in records:
                cur = conn.execute(
                    """INSERT OR IGNORE INTO ex10_exhibits
                       (accession, cik, form_type, doc_type, filename, description, sequence,
                        filing_url, found_at, markdown, markdown_status, filing_metadata, image_urls)
                       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
                    (
                        r.get("accession"), r.get("cik"), r.get("form_type"), r.get("doc_type"),
                        r.get("filename"), r.get("description"), r.get("sequence"),
                        r.get("filing_url"), r.get("found_at"), r.get("markdown") or None,
                        r.get("markdown_status") or None, r.get("filing_metadata") or None,
                        r.get("image_urls") or None,
                    ),
                )
                inserted += cur.rowcount
            conn.commit()
        return inserted

    def finalized_unmirrored(self, limit: int = 100, *, require_images: bool = True) -> list[dict[str, Any]]:
        # When image capture is disabled (no HF_TOKEN) image_urls stays NULL, so
        # require_images=False lets rows finalize on markdown + metadata alone.
        # The clause is a fixed internal literal (not user input) — no injection.
        image_clause = "AND image_urls IS NOT NULL" if require_images else ""
        with self.connect() as conn:
            rows = conn.execute(
                f"""SELECT id, accession, cik, form_type, doc_type, filename, description,
                          sequence, filing_url, found_at, source, size_bytes, detected_at,
                          json_extract(filing_metadata, '$.filed_at') AS filed_at,
                          markdown_status, filing_metadata, image_urls, markdown
                   FROM ex10_exhibits
                   WHERE COALESCE(mirrored, 0) = 0
                     AND markdown_status IN ('done', 'empty', 'error')
                     AND filing_metadata IS NOT NULL
                     {image_clause}
                   ORDER BY id ASC LIMIT ?""",
                (limit,),
            ).fetchall()
        return [dict(r) for r in rows]

    def mark_mirrored(self, ids: list[int]) -> None:
        if not ids:
            return
        with self.connect() as conn:
            conn.executemany("UPDATE ex10_exhibits SET mirrored = 1 WHERE id = ?", [(i,) for i in ids])
            conn.commit()

    def bump_image_attempts(self, exhibit_id: int) -> int:
        with self.connect() as conn:
            conn.execute("UPDATE ex10_exhibits SET image_attempts = COALESCE(image_attempts,0)+1 WHERE id = ?", (exhibit_id,))
            conn.commit()
            return conn.execute("SELECT image_attempts FROM ex10_exhibits WHERE id = ?", (exhibit_id,)).fetchone()[0]

    # --- reads --------------------------------------------------------------
    # List/search payloads only need a short excerpt, so we truncate the markdown
    # column in SQL (SUBSTR) instead of loading multi-MB contract bodies into
    # memory per row — important on small hosts. Detail reads use SELECT *.
    _SUMMARY_COLS = (
        "id, accession, cik, form_type, doc_type, filename, description, sequence, "
        "filing_url, found_at, markdown_status, filing_metadata, image_urls, "
        "SUBSTR(markdown, 1, 2000) AS markdown"
    )

    # Newest-by-actual-filing-time first: the displayed time is the SEC acceptance
    # timestamp (filing_metadata.filed_at, "YYYYMMDDHHMMSS" — lexical = chronological).
    # Rows not yet backfilled (NULL) sink below; found_at/id break ties.
    _LIST_ORDER = (
        "ORDER BY json_extract(filing_metadata, '$.filed_at') DESC NULLS LAST, "
        "found_at DESC, id DESC"
    )
    _LIST_ORDER_ASC = (
        "ORDER BY json_extract(filing_metadata, '$.filed_at') ASC NULLS LAST, "
        "found_at ASC, id ASC"
    )

    @staticmethod
    def _browse_where(form: str | None, cik: str | None, filer: str | None) -> tuple[str, list]:
        clauses, params = [], []
        if form:
            clauses.append("form_type = ?")
            params.append(form)
        if cik:
            clauses.append("cik = ?")
            params.append(cik)
        if filer:
            clauses.append("json_extract(filing_metadata, '$.company_name') LIKE ? COLLATE NOCASE")
            params.append(f"%{filer}%")
        where = ("WHERE " + " AND ".join(clauses)) if clauses else ""
        return where, params

    def recent_ex10(self, limit: int = 50, offset: int = 0, *, form: str | None = None,
                    cik: str | None = None, filer: str | None = None, oldest: bool = False) -> list[dict[str, Any]]:
        where, params = self._browse_where(form, cik, filer)
        order = self._LIST_ORDER_ASC if oldest else self._LIST_ORDER
        with self.connect() as conn:
            rows = conn.execute(
                f"SELECT {self._SUMMARY_COLS} FROM ex10_exhibits {where} {order} LIMIT ? OFFSET ?",
                (*params, limit, offset),
            ).fetchall()
        return [dict(r) for r in rows]

    def count_ex10(self, *, form: str | None = None, cik: str | None = None,
                   filer: str | None = None) -> int:
        where, params = self._browse_where(form, cik, filer)
        with self.connect() as conn:
            return conn.execute(
                f"SELECT COUNT(*) FROM ex10_exhibits {where}", params
            ).fetchone()[0]

    def form_facets(self) -> list[dict[str, Any]]:
        """Form-type counts for the Browse filter sidebar, most common first."""
        with self.connect() as conn:
            rows = conn.execute(
                """SELECT form_type, COUNT(*) AS count FROM ex10_exhibits
                   WHERE form_type IS NOT NULL AND form_type <> ''
                   GROUP BY form_type ORDER BY count DESC, form_type ASC"""
            ).fetchall()
        return [dict(r) for r in rows]

    def ex10_since(self, seconds: int) -> list[dict[str, Any]]:
        with self.connect() as conn:
            rows = conn.execute(
                f"""SELECT {self._SUMMARY_COLS} FROM ex10_exhibits
                    WHERE found_at >= datetime('now', ?)
                    {self._LIST_ORDER}""",
                (f"-{int(seconds)} seconds",),
            ).fetchall()
        return [dict(r) for r in rows]

    def stats(self) -> dict[str, Any]:
        """Aggregate counts for the stats endpoint."""
        with self.connect() as conn:
            total = conn.execute("SELECT COUNT(*) FROM ex10_exhibits").fetchone()[0]
            with_md = conn.execute(
                "SELECT COUNT(*) FROM ex10_exhibits WHERE markdown_status = 'done'"
            ).fetchone()[0]
            pending = conn.execute(
                "SELECT COUNT(*) FROM ex10_exhibits WHERE markdown_status IS NULL OR markdown_status = 'pending'"
            ).fetchone()[0]
            last_24h = conn.execute(
                "SELECT COUNT(*) FROM ex10_exhibits WHERE found_at >= datetime('now','-1 day')"
            ).fetchone()[0]
            by_doc = conn.execute(
                "SELECT doc_type, COUNT(*) c FROM ex10_exhibits GROUP BY doc_type ORDER BY c DESC"
            ).fetchall()
            by_form = conn.execute(
                "SELECT form_type, COUNT(*) c FROM ex10_exhibits GROUP BY form_type ORDER BY c DESC"
            ).fetchall()
        return {
            "total": total,
            "with_markdown": with_md,
            "pending_markdown": pending,
            "last_24h": last_24h,
            "by_doc_type": {r[0]: r[1] for r in by_doc},
            "by_form_type": {r[0]: r[1] for r in by_form},
        }

    @staticmethod
    def _like_term(query: str) -> str:
        """Escape LIKE wildcards so user-typed % and _ match literally."""
        escaped = query.replace("\\", "\\\\").replace("%", "\\%").replace("_", "\\_")
        return f"%{escaped}%"

    def search(self, query: str, limit: int = 20, offset: int = 0) -> list[dict[str, Any]]:
        """Case-insensitive substring search over description + markdown."""
        q = (query or "").strip()
        if not q:
            return []
        like = self._like_term(q)
        with self.connect() as conn:
            rows = conn.execute(
                f"""SELECT {self._SUMMARY_COLS} FROM ex10_exhibits
                    WHERE description LIKE ? ESCAPE '\\' COLLATE NOCASE
                       OR markdown LIKE ? ESCAPE '\\' COLLATE NOCASE
                    {self._LIST_ORDER} LIMIT ? OFFSET ?""",
                (like, like, limit, offset),
            ).fetchall()
        return [dict(r) for r in rows]

    def search_count(self, query: str) -> int:
        q = (query or "").strip()
        if not q:
            return 0
        like = self._like_term(q)
        with self.connect() as conn:
            return conn.execute(
                """SELECT COUNT(*) FROM ex10_exhibits
                   WHERE description LIKE ? ESCAPE '\\' COLLATE NOCASE
                      OR markdown LIKE ? ESCAPE '\\' COLLATE NOCASE""",
                (like, like),
            ).fetchone()[0]

    def exhibits_missing_filing_metadata(self, limit: int = 100) -> list[dict[str, Any]]:
        with self.connect() as conn:
            rows = conn.execute(
                """SELECT id, accession, cik, filename FROM ex10_exhibits
                   WHERE filing_metadata IS NULL
                      OR (filing_metadata <> '{}'
                          AND json_extract(filing_metadata, '$.filed_at') IS NULL)
                   ORDER BY found_at DESC, id DESC LIMIT ?""",
                (limit,),
            ).fetchall()
        return [dict(r) for r in rows]

    def exhibits_missing_markdown(self, limit: int = 100) -> list[dict[str, Any]]:
        """Rows awaiting markdown conversion.

        Keyed off ``markdown_status``: ``NULL``/``pending`` are eligible; ``done``,
        ``empty`` and ``error`` are terminal and never re-queued automatically.
        """
        with self.connect() as conn:
            rows = conn.execute(
                """SELECT * FROM ex10_exhibits
                   WHERE markdown_status IS NULL OR markdown_status = 'pending'
                   ORDER BY found_at DESC, id DESC LIMIT ?""",
                (limit,),
            ).fetchall()
        return [dict(r) for r in rows]
