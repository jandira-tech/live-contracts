"""Tests for the SQLite data-access layer, incl. the markdown column + migration."""
import sqlite3

import pytest

from sec_listener.db import Database


@pytest.fixture
def db(tmp_path):
    database = Database(str(tmp_path / "test.db"))
    database.init()
    return database


def test_init_creates_ex10_table_with_markdown_column(db):
    cols = db.column_names("ex10_exhibits")
    assert "markdown" in cols
    assert "filing_url" in cols
    assert "filing_metadata" in cols


def test_save_and_read_filing_metadata(db):
    meta = {"company_name": "Lamb Weston Holdings, Inc.", "period": "20260519",
            "items": ["Entry into a Material Definitive Agreement"]}
    db.save_ex10_exhibit(
        {"accession": "m-1", "cik": "1", "form_type": "8-K", "doc_type": "EX-10.1",
         "filename": "a.htm", "description": "", "sequence": "1", "url": "u"},
        markdown="body", filing_metadata=meta,
    )
    rows = db.recent_ex10()
    import json
    assert json.loads(rows[0]["filing_metadata"])["company_name"] == "Lamb Weston Holdings, Inc."


def test_missing_filing_metadata_and_update(db):
    db.save_ex10_exhibit(
        {"accession": "m-2", "cik": "1", "form_type": "8-K", "doc_type": "EX-10.1",
         "filename": "b.htm", "description": "", "sequence": "1", "url": "u"},
        markdown="body", filing_metadata=None,
    )
    missing = db.exhibits_missing_filing_metadata(limit=10)
    assert len(missing) == 1
    db.update_filing_metadata(missing[0]["id"], {"company_name": "X"})
    assert db.exhibits_missing_filing_metadata(limit=10) == []


def test_migration_adds_markdown_to_legacy_table(tmp_path):
    # Simulate a pre-existing DB that lacks the markdown column.
    path = str(tmp_path / "legacy.db")
    with sqlite3.connect(path) as conn:
        conn.execute(
            """CREATE TABLE ex10_exhibits (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                accession TEXT, cik TEXT, form_type TEXT, doc_type TEXT,
                filename TEXT, description TEXT, sequence TEXT, filing_url TEXT,
                found_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(accession, doc_type, filename)
            )"""
        )
        conn.execute(
            "INSERT INTO ex10_exhibits (accession, doc_type, filename) VALUES ('a-1','EX-10.1','x.htm')"
        )
        conn.commit()

    db = Database(path)
    db.init()  # must be idempotent and add the column without losing data

    assert "markdown" in db.column_names("ex10_exhibits")
    with sqlite3.connect(path) as conn:
        n = conn.execute("SELECT COUNT(*) FROM ex10_exhibits").fetchone()[0]
    assert n == 1


def test_save_ex10_exhibit_persists_markdown(db):
    db.save_ex10_exhibit(
        {
            "accession": "0001-26-000001",
            "cik": "123",
            "form_type": "8-K",
            "doc_type": "EX-10.1",
            "filename": "ex10_1.htm",
            "description": "Material Contract",
            "sequence": "2",
            "url": "https://sec.gov/x.txt",
        },
        markdown="# Material Contract\n\nbody",
    )
    rows = db.recent_ex10(limit=10)
    assert len(rows) == 1
    assert rows[0]["markdown"].startswith("# Material Contract")
    assert rows[0]["doc_type"] == "EX-10.1"


def test_exhibits_missing_markdown_and_update(db):
    db.save_ex10_exhibit(
        {
            "accession": "0001-26-000002",
            "cik": "1",
            "form_type": "8-K",
            "doc_type": "EX-10.1",
            "filename": "a.htm",
            "description": "",
            "sequence": "1",
            "url": "u",
        },
        markdown=None,
    )
    missing = db.exhibits_missing_markdown(limit=10)
    assert len(missing) == 1
    ex_id = missing[0]["id"]

    db.update_markdown(ex_id, "converted text")
    assert db.exhibits_missing_markdown(limit=10) == []


def test_since_returns_only_recent_rows(db):
    db.save_ex10_exhibit(
        {
            "accession": "0001-26-000003",
            "cik": "1",
            "form_type": "8-K",
            "doc_type": "EX-10.1",
            "filename": "b.htm",
            "description": "",
            "sequence": "1",
            "url": "u",
        },
        markdown="x",
    )
    # Force an older row by backdating its found_at one hour.
    with db.connect() as conn:
        conn.execute(
            """INSERT INTO ex10_exhibits
               (accession, doc_type, filename, found_at, markdown, markdown_status)
               VALUES ('old-1','EX-10.1','old.htm', datetime('now','-3600 seconds'), 'y', 'done')"""
        )
        conn.commit()

    # The fresh row is within the last 60s; the backdated one is not.
    recent = db.ex10_since(seconds=60)
    assert len(recent) == 1
    assert recent[0]["accession"] == "0001-26-000003"
    # A wide window picks up both.
    assert len(db.ex10_since(seconds=7200)) == 2


def test_empty_filing_metadata_persists_not_null(db):
    """An empty {} (a processed-but-headerless filing) must persist as '{}', not NULL,
    so the backfill worker does not re-select it as missing forever."""
    db.save_ex10_exhibit(
        {"accession": "em-1", "cik": "1", "form_type": "8-K", "doc_type": "EX-10.1",
         "filename": "e.htm", "description": "", "sequence": "1", "url": "u"},
        markdown="body", filing_metadata={},
    )
    rows = db.recent_ex10()
    assert rows[0]["filing_metadata"] == "{}"
    assert db.exhibits_missing_filing_metadata(limit=10) == []
