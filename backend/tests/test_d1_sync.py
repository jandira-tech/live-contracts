import pytest
from sec_listener.db import Database
from sec_listener.d1_sync import push_finalized, to_ingest_record


@pytest.fixture
def db(tmp_path):
    d = Database(str(tmp_path / "s.db")); d.init(); return d


def _finalize(d, acc):
    d.save_ex10_exhibit({"accession": acc, "cik": "1", "form_type": "8-K", "doc_type": "EX-10.1",
                         "filename": f"{acc}.htm", "description": "", "sequence": "1", "url": "u"},
                        markdown="body", filing_metadata={"filed_at": "20260501120000"})
    row = [r for r in d.recent_ex10() if r["accession"] == acc][0]
    d.update_image_urls(row["id"], [])


def test_to_ingest_record_extracts_filed_at(db):
    _finalize(db, "a")
    rec = to_ingest_record(db.finalized_unmirrored()[0])
    assert rec["filed_at"] == "20260501120000" and rec["accession"] == "a" and rec["markdown"] == "body"


def test_push_finalized_posts_and_marks(db):
    _finalize(db, "a"); _finalize(db, "b")
    posted = []
    # The ingest route returns the accepted unique IDs (not accessions).
    def poster(url, rows): posted.append((url, [r["accession"] for r in rows])); return [r["id"] for r in rows]
    assert push_finalized(db, "http://w/api/ingest", "KEY", batch=50, poster=poster) == 2
    assert posted == [("http://w/api/ingest", ["a", "b"])]
    assert db.finalized_unmirrored() == []


def test_push_finalized_marks_only_pushed_ids_not_whole_accession(db):
    """accession is NOT unique — marking by id must not sweep a sibling row."""
    # Two distinct exhibits sharing accession 'dup' (different doc_type/filename).
    for doc, fn in (("EX-10.1", "x1.htm"), ("EX-10.2", "x2.htm")):
        db.save_ex10_exhibit({"accession": "dup", "cik": "1", "form_type": "8-K", "doc_type": doc,
                              "filename": fn, "description": "", "sequence": "1", "url": "u"},
                             markdown="body", filing_metadata={"filed_at": "20260501120000"})
    for r in db.recent_ex10():
        db.update_image_urls(r["id"], [])
    # Push only the first row (batch=1); only its id is accepted.
    def poster(url, rows): return [rows[0]["id"]]
    assert push_finalized(db, "http://w/api/ingest", "KEY", batch=1, poster=poster) == 1
    # The sibling (same accession, not pushed) must remain unmirrored.
    remaining = db.finalized_unmirrored()
    assert len(remaining) == 1


def test_push_finalized_failure_leaves_unmirrored(db):
    _finalize(db, "a")
    def poster(url, rows): raise RuntimeError("503")
    assert push_finalized(db, "http://w/api/ingest", "KEY", poster=poster) == 0
    assert [r["accession"] for r in db.finalized_unmirrored()] == ["a"]


def test_finalized_unmirrored_without_require_images(db):
    """When image capture is disabled (no HF_TOKEN), rows finalize without image_urls."""
    # markdown done + metadata present, but image_urls left NULL (never captured).
    db.save_ex10_exhibit({"accession": "noimg", "cik": "1", "form_type": "8-K", "doc_type": "EX-10.1",
                          "filename": "n.htm", "description": "", "sequence": "1", "url": "u"},
                         markdown="body", filing_metadata={"filed_at": "20260501120000"})
    assert db.finalized_unmirrored() == []                       # image_urls NULL -> not finalized
    assert len(db.finalized_unmirrored(require_images=False)) == 1  # finalized when images disabled
