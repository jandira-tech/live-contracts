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
    db.update_filing_metadata(missing[0]["id"], {"company_name": "X", "filed_at": "20260519120000"})
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


def test_missing_filing_metadata_includes_legacy_rows_without_filed_at(db):
    # Legacy row: filing_metadata parsed before filed_at existed (no such key).
    db.save_ex10_exhibit(
        {"accession": "legacy", "cik": "1", "form_type": "8-K", "doc_type": "EX-10.1",
         "filename": "l.htm", "description": "", "sequence": "1", "url": "u"},
        markdown="x", filing_metadata={"company_name": "Old Co"},
    )
    # Current row: already carries filed_at.
    db.save_ex10_exhibit(
        {"accession": "current", "cik": "1", "form_type": "8-K", "doc_type": "EX-10.1",
         "filename": "c.htm", "description": "", "sequence": "1", "url": "u"},
        markdown="x", filing_metadata={"company_name": "New Co", "filed_at": "20260526113637"},
    )
    accs = {m["accession"] for m in db.exhibits_missing_filing_metadata(limit=10)}
    assert "legacy" in accs       # re-fetched to add filed_at
    assert "current" not in accs  # already has filed_at -> skipped


def test_missing_filing_metadata_robust_to_filed_at_substring(db):
    # A field VALUE containing the literal '"filed_at"' must not be mistaken for the key.
    db.save_ex10_exhibit(
        {"accession": "tricky", "cik": "1", "form_type": "8-K", "doc_type": "EX-10.1",
         "filename": "t.htm", "description": "", "sequence": "1", "url": "u"},
        markdown="x", filing_metadata={"note": "filed_at"},  # no real filed_at key
    )
    accs = {m["accession"] for m in db.exhibits_missing_filing_metadata(limit=10)}
    assert "tricky" in accs  # still needs backfill (json key absent)


def test_insert_exhibits_bulk_inserts_and_ignores_dups(db):
    recs = [
        {"accession": "b-1", "cik": "1", "form_type": "8-K", "doc_type": "EX-10.1", "filename": "a.htm",
         "description": "D", "sequence": "1", "filing_url": "u", "found_at": "2026-05-26 10:00:00",
         "markdown": "body", "markdown_status": "done", "filing_metadata": '{"filed_at":"x"}'},
        {"accession": "b-2", "cik": "1", "form_type": "8-K", "doc_type": "EX-10.2", "filename": "b.htm",
         "description": "", "sequence": "2", "filing_url": "u", "found_at": "2026-05-26 11:00:00",
         "markdown": "", "markdown_status": "empty", "filing_metadata": None},
    ]
    assert db.insert_exhibits_bulk(recs) == 2
    assert db.count_ex10() == 2
    assert db.insert_exhibits_bulk(recs) == 0  # idempotent — UNIQUE(accession,doc_type,filename)
    rows = {r["accession"]: r for r in db.recent_ex10()}
    assert rows["b-1"]["found_at"] == "2026-05-26 10:00:00"  # preserved, not now()
    assert rows["b-1"]["markdown_status"] == "done"


def test_insert_exhibits_bulk_maps_empty_to_null(db):
    # Synced parquet coerces missing fields to "" — restore must map them back to
    # NULL so backfill queries still pick the row up.
    db.insert_exhibits_bulk([
        {"accession": "r1", "cik": "1", "form_type": "8-K", "doc_type": "EX-10.1", "filename": "r.htm",
         "description": "", "sequence": "1", "filing_url": "u", "found_at": "2026-05-26 10:00:00",
         "markdown": "", "markdown_status": "", "filing_metadata": "", "image_urls": ""},
    ])
    assert [r["accession"] for r in db.exhibits_missing_markdown(limit=10)] == ["r1"]  # "" -> NULL -> pending
    row = db.recent_ex10()[0]
    assert row["filing_metadata"] is None
    assert row["image_urls"] is None  # "" -> NULL so exhibits_pending_images still selects it


def test_image_urls_column_update_and_pending(db):
    import json
    db.save_ex10_exhibit(
        {"accession": "img-1", "cik": "1", "form_type": "8-K", "doc_type": "EX-10.1",
         "filename": "i.htm", "description": "", "sequence": "1", "url": "u"},
        markdown="(ex10-1_001.jpg) (ex10-1_002.jpg)",  # image-only body, status done
    )
    db.save_ex10_exhibit(
        {"accession": "txt-1", "cik": "1", "form_type": "8-K", "doc_type": "EX-10.2",
         "filename": "t.htm", "description": "", "sequence": "1", "url": "u"},
        markdown="Real contract text only, no images",
    )
    assert "image_urls" in db.column_names("ex10_exhibits")
    # pending = done + image_urls NULL + has an image ref
    pending = {r["accession"] for r in db.exhibits_pending_images(limit=10)}
    assert pending == {"img-1"}  # txt-1 has no image ref
    pid = next(r["id"] for r in db.exhibits_pending_images(limit=10))
    db.update_image_urls(pid, ["https://hf/x/ex10-1_001.jpg"])
    # now stored + no longer pending
    assert db.exhibits_pending_images(limit=10) == []
    row = next(r for r in db.recent_ex10() if r["accession"] == "img-1")
    assert json.loads(row["image_urls"]) == ["https://hf/x/ex10-1_001.jpg"]


def test_recent_ex10_orders_by_filed_at_over_found_at(db):
    # A captured later (newer found_at) but FILED earlier; B captured earlier but FILED later.
    db.insert_exhibits_bulk([
        {"accession": "A", "cik": "1", "form_type": "8-K", "doc_type": "EX-10.1", "filename": "a.htm",
         "description": "", "sequence": "1", "filing_url": "u", "found_at": "2026-05-26 22:00:00",
         "markdown": "x", "markdown_status": "done", "filing_metadata": '{"filed_at":"20260526170000"}'},
        {"accession": "B", "cik": "1", "form_type": "8-K", "doc_type": "EX-10.2", "filename": "b.htm",
         "description": "", "sequence": "1", "filing_url": "u", "found_at": "2026-05-26 21:00:00",
         "markdown": "x", "markdown_status": "done", "filing_metadata": '{"filed_at":"20260526180000"}'},
    ])
    # Newest by *filing* time first -> B (18:00) then A (17:00), not found_at order.
    assert [r["accession"] for r in db.recent_ex10()] == ["B", "A"]


def test_recent_ex10_filed_at_nulls_sink_below_filed(db):
    # Row with filed_at ranks above a row without it (older, unbackfilled).
    db.insert_exhibits_bulk([
        {"accession": "nofiled", "cik": "1", "form_type": "8-K", "doc_type": "EX-10.1", "filename": "n.htm",
         "description": "", "sequence": "1", "filing_url": "u", "found_at": "2026-05-26 23:00:00",
         "markdown": "x", "markdown_status": "done", "filing_metadata": None},
        {"accession": "hasfiled", "cik": "1", "form_type": "8-K", "doc_type": "EX-10.2", "filename": "h.htm",
         "description": "", "sequence": "1", "filing_url": "u", "found_at": "2026-05-26 20:00:00",
         "markdown": "x", "markdown_status": "done", "filing_metadata": '{"filed_at":"20260526190000"}'},
    ])
    assert [r["accession"] for r in db.recent_ex10()] == ["hasfiled", "nofiled"]


def test_filed_at_expression_index_created(db):
    # An expression index backs the filed_at ORDER BY (avoids JSON parse per row).
    with db.connect() as conn:
        row = conn.execute(
            "SELECT sql FROM sqlite_master WHERE type='index' AND name='idx_ex10_filed_at'"
        ).fetchone()
    assert row is not None and "filed_at" in row[0]
def _bulk(db, *rows):
    db.insert_exhibits_bulk([
        {"accession": a, "cik": c, "form_type": ft, "doc_type": "EX-10.1", "filename": f"{a}.htm",
         "description": "", "sequence": "1", "filing_url": "u", "found_at": fa, "markdown": "x",
         "markdown_status": "done", "filing_metadata": fm}
        for (a, c, ft, fa, fm) in rows
    ])


def test_browse_filters_by_form_cik_and_filer(db):
    _bulk(db,
        ("a", "111", "8-K", "2026-05-26 10:00:00", '{"filed_at":"20260526100000","company_name":"Acme Corp"}'),
        ("b", "222", "10-Q", "2026-05-26 11:00:00", '{"filed_at":"20260526110000","company_name":"Beta LLC"}'),
        ("c", "111", "8-K", "2026-05-26 12:00:00", '{"filed_at":"20260526120000","company_name":"Acme Corp"}'),
    )
    assert {r["accession"] for r in db.recent_ex10(form="8-K")} == {"a", "c"}
    assert {r["accession"] for r in db.recent_ex10(cik="222")} == {"b"}
    assert {r["accession"] for r in db.recent_ex10(filer="acme")} == {"a", "c"}  # case-insensitive
    assert db.count_ex10(form="8-K") == 2
    assert db.count_ex10(cik="222") == 1


def test_browse_sort_oldest_reverses(db):
    _bulk(db,
        ("old", "1", "8-K", "2026-05-26 10:00:00", '{"filed_at":"20260526100000"}'),
        ("new", "1", "8-K", "2026-05-26 12:00:00", '{"filed_at":"20260526120000"}'),
    )
    assert [r["accession"] for r in db.recent_ex10()] == ["new", "old"]            # newest default
    assert [r["accession"] for r in db.recent_ex10(oldest=True)] == ["old", "new"]  # oldest first


def test_form_facets_counts(db):
    _bulk(db,
        ("a", "1", "8-K", "2026-05-26 10:00:00", "{}"),
        ("b", "1", "8-K", "2026-05-26 11:00:00", "{}"),
        ("c", "1", "10-Q", "2026-05-26 12:00:00", "{}"),
    )
    facets = dict((f["form_type"], f["count"]) for f in db.form_facets())
    assert facets == {"8-K": 2, "10-Q": 1}


def test_finalized_unmirrored_selects_only_complete_rows(tmp_path):
    from sec_listener.db import Database
    d = Database(str(tmp_path / "f.db")); d.init()
    d.save_ex10_exhibit({"accession":"a","cik":"1","form_type":"8-K","doc_type":"EX-10.1",
                         "filename":"a.htm","description":"","sequence":"1","url":"u"},
                        markdown="body", filing_metadata={"filed_at":"20260501120000"})
    d.update_image_urls(d.recent_ex10()[0]["id"], [])         # images resolved -> finalized
    d.save_ex10_exhibit({"accession":"b","cik":"1","form_type":"8-K","doc_type":"EX-10.1",
                         "filename":"b.htm","description":"","sequence":"1","url":"u"}, markdown=None)
    fin = d.finalized_unmirrored(limit=10)
    assert [r["accession"] for r in fin] == ["a"]
    d.mark_mirrored([r["id"] for r in fin])
    assert d.finalized_unmirrored(limit=10) == []
