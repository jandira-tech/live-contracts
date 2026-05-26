"""Tests for the markdown backfill worker."""
import pytest

from sec_listener.db import Database
from sec_listener.worker import BackfillWorker


@pytest.fixture
def db(tmp_path):
    d = Database(str(tmp_path / "w.db"))
    d.init()
    return d


def _add(db, accession, status_none=True):
    db.save_ex10_exhibit(
        {
            "accession": accession,
            "cik": "1",
            "form_type": "8-K",
            "doc_type": "EX-10.1",
            "filename": f"{accession}.htm",
            "description": "",
            "sequence": "1",
            "url": "u",
        },
        markdown=None,  # status -> pending
    )


def test_backfill_converts_pending_rows(db):
    _add(db, "a-1")
    _add(db, "a-2")

    def fetcher(accession, cik, filename):
        return f"<h1>{accession}</h1>"

    worker = BackfillWorker(db, fetcher=fetcher)
    n = worker.backfill_batch(limit=10)
    assert n == 2
    rows = db.recent_ex10()
    assert all(r["markdown_status"] == "done" for r in rows)
    assert all(r["markdown"] for r in rows)
    # nothing left to do
    assert db.exhibits_missing_markdown(limit=10) == []


def test_empty_content_marked_empty_and_not_retried(db):
    _add(db, "a-3")

    def fetcher(accession, cik, filename):
        return ""  # no content available

    worker = BackfillWorker(db, fetcher=fetcher)
    worker.backfill_batch(limit=10)
    rows = db.recent_ex10()
    assert rows[0]["markdown_status"] == "empty"
    # excluded from future backfill attempts
    assert db.exhibits_missing_markdown(limit=10) == []


def test_metadata_backfill_fills_missing(db):
    _add(db, "fm-1")
    _add(db, "fm-2")
    assert len(db.exhibits_missing_filing_metadata(limit=10)) == 2

    def meta_fetcher(accession, cik):
        return {"company_name": f"Co {accession}", "period": "20260519", "items": []}

    worker = BackfillWorker(db, metadata_fetcher=meta_fetcher)
    n = worker.backfill_metadata_batch(limit=10)
    assert n == 2
    assert db.exhibits_missing_filing_metadata(limit=10) == []
    import json
    rows = db.recent_ex10()
    assert all(json.loads(r["filing_metadata"])["company_name"].startswith("Co ") for r in rows)


def test_metadata_backfill_error_is_isolated(db):
    _add(db, "ok-meta")
    _add(db, "bad-meta")

    def meta_fetcher(accession, cik):
        if accession == "bad-meta":
            raise RuntimeError("404")
        return {"company_name": "Good"}

    worker = BackfillWorker(db, metadata_fetcher=meta_fetcher)
    n = worker.backfill_metadata_batch(limit=10)
    assert n == 1  # only the good one counted
    # bad one is marked (empty) so it is not retried forever
    assert db.exhibits_missing_filing_metadata(limit=10) == []


def test_fetch_error_marked_error_and_isolated(db):
    _add(db, "ok-1")
    _add(db, "bad-1")

    def fetcher(accession, cik, filename):
        if accession == "bad-1":
            raise RuntimeError("404")
        return "<p>ok</p>"

    worker = BackfillWorker(db, fetcher=fetcher)
    n = worker.backfill_batch(limit=10)
    assert n == 1  # only the good one counted
    statuses = {r["accession"]: r["markdown_status"] for r in db.recent_ex10()}
    assert statuses["ok-1"] == "done"
    assert statuses["bad-1"] == "error"
    # error rows are not retried automatically
    assert db.exhibits_missing_markdown(limit=10) == []


def test_metadata_backfill_throttles_between_requests(db):
    """Each metadata fetch must be preceded by a rate-limit delay (SEC 10 RPS)."""
    _add(db, "rl-1")
    _add(db, "rl-2")
    slept: list[float] = []
    worker = BackfillWorker(
        db,
        metadata_fetcher=lambda a, c: {"company_name": "X"},
        sleep_fn=slept.append,
    )
    worker.backfill_metadata_batch(limit=10)
    # one throttle per row, at the configured delay
    assert slept == [worker.request_delay, worker.request_delay]
    assert all(d > 0 for d in slept)


def test_markdown_backfill_throttles_between_requests(db):
    """The markdown backfill loop also fetches sec.gov per row and must throttle (SEC 10 RPS)."""
    _add(db, "mb-1")
    _add(db, "mb-2")
    slept: list[float] = []
    worker = BackfillWorker(
        db,
        fetcher=lambda a, c, f: "<p>doc</p>",
        convert_fn=lambda t: "md",
        sleep_fn=slept.append,
    )
    worker.backfill_batch(limit=10)
    assert slept == [worker.request_delay, worker.request_delay]


def test_negative_request_delay_does_not_sleep(db):
    """A misconfigured negative delay must not reach time.sleep (would ValueError)."""
    _add(db, "neg-1")
    slept = []
    worker = BackfillWorker(db, metadata_fetcher=lambda a, c: {"filed_at": "x"},
                            request_delay=-1.0, sleep_fn=slept.append)
    worker.backfill_metadata_batch(limit=10)
    assert slept == []  # guarded by request_delay > 0
