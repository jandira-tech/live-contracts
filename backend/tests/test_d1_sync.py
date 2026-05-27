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
    def poster(url, rows): posted.append((url, [r["accession"] for r in rows])); return [r["accession"] for r in rows]
    assert push_finalized(db, "http://w/api/ingest", "KEY", batch=50, poster=poster) == 2
    assert posted == [("http://w/api/ingest", ["a", "b"])]
    assert db.finalized_unmirrored() == []


def test_push_finalized_failure_leaves_unmirrored(db):
    _finalize(db, "a")
    def poster(url, rows): raise RuntimeError("503")
    assert push_finalized(db, "http://w/api/ingest", "KEY", poster=poster) == 0
    assert [r["accession"] for r in db.finalized_unmirrored()] == ["a"]
