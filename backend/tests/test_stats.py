"""Tests for aggregate stats (db.stats + /api/stats)."""
import pytest
from fastapi.testclient import TestClient

from sec_listener.api import create_app
from sec_listener.db import Database


@pytest.fixture
def db(tmp_path):
    d = Database(str(tmp_path / "stats.db"))
    d.init()
    return d


def _add(db, accession, doc_type, form_type, markdown="x"):
    db.save_ex10_exhibit(
        {
            "accession": accession,
            "cik": "1",
            "form_type": form_type,
            "doc_type": doc_type,
            "filename": f"{accession}.htm",
            "description": "",
            "sequence": "1",
            "url": "u",
        },
        markdown=markdown,
    )


def test_stats_aggregates_totals_and_breakdowns(db):
    _add(db, "a-1", "EX-10.1", "8-K")
    _add(db, "a-2", "EX-10.1", "8-K")
    _add(db, "a-3", "EX-10.2", "10-Q")
    _add(db, "a-4", "EX-10.1", "S-1", markdown=None)  # pending markdown

    stats = db.stats()
    assert stats["total"] == 4
    assert stats["with_markdown"] == 3
    assert stats["pending_markdown"] == 1
    assert stats["by_doc_type"]["EX-10.1"] == 3
    assert stats["by_doc_type"]["EX-10.2"] == 1
    assert stats["by_form_type"]["8-K"] == 2
    assert stats["last_24h"] == 4  # all just inserted


def test_stats_last_24h_excludes_old_rows(db):
    _add(db, "fresh", "EX-10.1", "8-K")
    with db.connect() as conn:
        conn.execute(
            """INSERT INTO ex10_exhibits (accession, doc_type, form_type, filename,
               found_at, markdown, markdown_status)
               VALUES ('old','EX-10.1','8-K','o.htm', datetime('now','-2 days'), 'm', 'done')"""
        )
        conn.commit()
    stats = db.stats()
    assert stats["total"] == 2
    assert stats["last_24h"] == 1


def test_api_stats_endpoint(db):
    _add(db, "a-1", "EX-10.1", "8-K")
    client = TestClient(create_app(db, api_key=None))
    r = client.get("/api/stats")
    assert r.status_code == 200
    body = r.json()
    assert body["total"] == 1
    assert "by_doc_type" in body and "by_form_type" in body
    assert "stale-while-revalidate" in r.headers.get("cache-control", "")


def test_api_stats_requires_key_when_set(db):
    client = TestClient(create_app(db, api_key="k"))
    assert client.get("/api/stats").status_code == 401
    assert client.get("/api/stats", headers={"X-API-Key": "k"}).status_code == 200
