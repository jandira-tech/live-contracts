"""Tests for the internal read-only FastAPI app."""
import pytest
from fastapi.testclient import TestClient

from sec_listener.api import create_app
from sec_listener.db import Database


def _seed(db: Database, n: int):
    for i in range(n):
        db.save_ex10_exhibit(
            {
                "accession": f"acc-{i:04d}",
                "cik": str(i),
                "form_type": "8-K",
                "doc_type": "EX-10.1",
                "filename": f"ex10_{i}.htm",
                "description": f"Contract {i}",
                "sequence": "2",
                "url": f"https://sec.gov/{i}.txt",
            },
            markdown=f"# Contract {i}\n\nbody {i}",
        )


@pytest.fixture
def db(tmp_path):
    d = Database(str(tmp_path / "api.db"))
    d.init()
    return d


def test_health_ok(db):
    client = TestClient(create_app(db, api_key=None))
    r = client.get("/health")
    assert r.status_code == 200
    assert r.json()["status"] == "ok"


def test_list_is_paginated(db):
    _seed(db, 25)
    client = TestClient(create_app(db, api_key=None))
    r = client.get("/api/ex10", params={"page": 1, "page_size": 10})
    assert r.status_code == 200
    body = r.json()
    assert body["total"] == 25
    assert body["page"] == 1
    assert body["page_size"] == 10
    assert body["total_pages"] == 3
    assert len(body["items"]) == 10
    # second page
    r2 = client.get("/api/ex10", params={"page": 3, "page_size": 10})
    assert len(r2.json()["items"]) == 5


def test_since_endpoint_returns_recent(db):
    _seed(db, 3)
    client = TestClient(create_app(db, api_key=None))
    r = client.get("/api/ex10/since", params={"seconds": 60})
    assert r.status_code == 200
    body = r.json()
    assert body["window_seconds"] == 60
    assert body["count"] == 3
    assert len(body["items"]) == 3


def test_detail_returns_markdown_and_404(db):
    _seed(db, 1)
    client = TestClient(create_app(db, api_key=None))
    rows = db.recent_ex10()
    ex_id = rows[0]["id"]
    r = client.get(f"/api/ex10/{ex_id}")
    assert r.status_code == 200
    assert r.json()["markdown"].startswith("# Contract 0")
    assert client.get("/api/ex10/999999").status_code == 404


def test_api_key_required_when_configured(db):
    _seed(db, 1)
    client = TestClient(create_app(db, api_key="s3cret"))
    # health stays open for liveness probes
    assert client.get("/health").status_code == 200
    # data endpoints require the key
    assert client.get("/api/ex10").status_code == 401
    ok = client.get("/api/ex10", headers={"X-API-Key": "s3cret"})
    assert ok.status_code == 200
    bad = client.get("/api/ex10", headers={"X-API-Key": "wrong"})
    assert bad.status_code == 401


def test_cache_headers_present_for_swr(db):
    _seed(db, 1)
    client = TestClient(create_app(db, api_key=None))
    r = client.get("/api/ex10/since", params={"seconds": 60})
    # stale-while-revalidate hint for the CDN edge
    assert "stale-while-revalidate" in r.headers.get("cache-control", "")


def test_parse_filing_rejects_non_dict_json():
    """_parse_filing must return {} for valid-but-non-object JSON, so downstream
    .get() calls never hit a list/str/number."""
    from sec_listener.api import _parse_filing
    assert _parse_filing(None) == {}
    assert _parse_filing("") == {}
    assert _parse_filing("[1, 2]") == {}
    assert _parse_filing('"a string"') == {}
    assert _parse_filing("42") == {}
    assert _parse_filing('{"company_name": "X"}') == {"company_name": "X"}
