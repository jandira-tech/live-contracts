"""Tests for the Listener orchestration (process_filing) with injected fakes."""
import pytest

from sec_listener.config import Config
from sec_listener.db import Database
from sec_listener.listener import Listener


@pytest.fixture
def listener(tmp_path):
    db = Database(str(tmp_path / "l.db"))
    db.init()
    cfg = Config(db_path=db.path, convert_markdown=True)
    return Listener(cfg, db)


def _filing(acc="0001-26-000001", form="8-K", cik="42"):
    return {"accession": acc, "cik": cik, "form_type": form,
            "filing_date": "2026-05-25", "summary": "Filed", "url": "u"}


def test_process_new_filing_saves_ex10_with_markdown(listener):
    def extractor(accession, cik):
        ex10 = [{"type": "EX-10.1", "filename": "a.htm", "description": "Contract",
                 "sequence": "2", "content": b"<h1>Deal</h1><p>terms</p>"}]
        other = [{"type": "EX-99.1", "filename": "p.htm", "description": "", "sequence": "3"}]
        return ex10, other, "https://sec.gov/x.txt", {"company_name": "Acme Corp", "period": "20260519"}

    listener.extractor = extractor
    ex10_count, other_count = listener.process_filing(_filing())

    assert ex10_count == 1
    assert other_count == 1
    rows = listener.db.recent_ex10()
    assert len(rows) == 1
    assert "Deal" in rows[0]["markdown"]
    assert rows[0]["filing_url"] == "https://sec.gov/x.txt"
    import json
    assert json.loads(rows[0]["filing_metadata"])["company_name"] == "Acme Corp"


def test_already_seen_filing_is_skipped(listener):
    listener.db.mark_accession_seen("dup-1", "8-K", "1")
    called = {"n": 0}

    def extractor(accession, cik):
        called["n"] += 1
        return [], [], "u", {}

    listener.extractor = extractor
    out = listener.process_filing(_filing(acc="dup-1"))
    assert out == (0, 0)
    assert called["n"] == 0  # extractor never invoked for seen accession


def test_ex10_without_content_saved_with_empty_markdown(listener):
    def extractor(accession, cik):
        return [{"type": "EX-10.2", "filename": "n.htm", "description": "",
                 "sequence": "1", "content": None}], [], "u", {}

    listener.extractor = extractor
    listener.process_filing(_filing(acc="0001-26-000009"))
    rows = listener.db.recent_ex10()
    assert len(rows) == 1
    assert (rows[0]["markdown"] or "") == ""


def test_extractor_failure_does_not_crash(listener):
    def extractor(accession, cik):
        raise RuntimeError("network down")

    listener.extractor = extractor
    # Robustness: a failing extraction returns zero counts, does not raise.
    out = listener.process_filing(_filing(acc="0001-26-000010"))
    assert out == (0, 0)
    # Accession is still marked seen so we don't hammer a broken filing forever.
    assert listener.db.is_accession_seen("0001-26-000010")
