"""Tests for live full-text search (db.search + /api/search)."""
import pytest
from fastapi.testclient import TestClient

from sec_listener.api import create_app
from sec_listener.db import Database


@pytest.fixture
def db(tmp_path):
    d = Database(str(tmp_path / "search.db"))
    d.init()
    return d


def _add(db, accession, doc_type, description, markdown):
    db.save_ex10_exhibit(
        {
            "accession": accession, "cik": "1", "form_type": "8-K",
            "doc_type": doc_type, "filename": f"{accession}.htm",
            "description": description, "sequence": "1", "url": "u",
        },
        markdown=markdown,
    )


def test_search_matches_description_and_markdown_case_insensitive(db):
    _add(db, "a-1", "EX-10.1", "Tax Receivable Agreement", "body about taxes")
    _add(db, "a-2", "EX-10.2", "Employment Agreement", "salary and EQUITY terms")
    _add(db, "a-3", "EX-10.3", "Lease", "nothing relevant here")

    # match in description
    res = db.search("receivable", limit=10, offset=0)
    assert [r["accession"] for r in res] == ["a-1"]
    # match in markdown body, case-insensitive
    res = db.search("equity", limit=10, offset=0)
    assert [r["accession"] for r in res] == ["a-2"]
    # no match
    assert db.search("zzz-no-match", limit=10, offset=0) == []


def test_search_count_and_pagination(db):
    for i in range(5):
        _add(db, f"agr-{i}", "EX-10.1", f"Master Agreement {i}", "agreement body")
    assert db.search_count("agreement") == 5
    page1 = db.search("agreement", limit=2, offset=0)
    page2 = db.search("agreement", limit=2, offset=2)
    assert len(page1) == 2 and len(page2) == 2
    assert {r["accession"] for r in page1}.isdisjoint({r["accession"] for r in page2})


def test_api_search_endpoint(db):
    _add(db, "a-1", "EX-10.1", "Credit Agreement", "revolving credit facility")
    client = TestClient(create_app(db, api_key=None))
    r = client.get("/api/search", params={"q": "credit", "page": 1, "page_size": 10})
    assert r.status_code == 200
    body = r.json()
    assert body["query"] == "credit"
    assert body["total"] == 1
    assert body["items"][0]["accession"] == "a-1"
    assert "stale-while-revalidate" in r.headers.get("cache-control", "")


def test_api_search_blank_query_returns_empty(db):
    _add(db, "a-1", "EX-10.1", "Credit Agreement", "x")
    client = TestClient(create_app(db, api_key=None))
    r = client.get("/api/search", params={"q": "  "})
    assert r.status_code == 200
    assert r.json()["total"] == 0
    assert r.json()["items"] == []
