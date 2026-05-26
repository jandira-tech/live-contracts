"""Tests for the parallel Hugging Face dataset sink (snapshot of ex10_exhibits)."""
from sec_listener.db import Database
from sec_listener import hf_sync


def _db(tmp_path):
    d = Database(str(tmp_path / "s.db"))
    d.init()
    return d


def _add(db, accession, markdown=None, meta=None):
    db.save_ex10_exhibit(
        {"accession": accession, "cik": "1", "form_type": "8-K", "doc_type": "EX-10.1",
         "filename": f"{accession}.htm", "description": "Deal", "sequence": "1", "url": "u"},
        markdown=markdown, filing_metadata=meta,
    )


def test_to_records_adds_has_markdown_and_is_json_safe():
    rows = [
        {"id": 1, "accession": "a", "markdown": "# body", "filing_metadata": '{"company_name":"X"}'},
        {"id": 2, "accession": "b", "markdown": None, "filing_metadata": None},
    ]
    recs = hf_sync.to_records(rows)
    assert recs[0]["has_markdown"] is True and recs[0]["markdown"] == "# body"
    assert recs[1]["has_markdown"] is False and recs[1]["markdown"] == ""
    # Never None — parquet/SQL friendly; missing fields default to "".
    assert recs[1]["filing_metadata"] == "" and recs[1]["cik"] == ""
    assert recs[0]["filing_metadata"] == '{"company_name":"X"}'


def test_sync_is_noop_without_token(tmp_path):
    db = _db(tmp_path)
    _add(db, "x-1")
    calls = []
    n = hf_sync.sync_exhibits(db, "arthrod/sec-ex10-exhibits", token="",
                              writer=lambda *a: calls.append("w"), uploader=lambda *a: calls.append("u"))
    assert n == 0 and calls == []  # SQLite remains the sole store (plan B)


def test_sync_writes_all_rows_and_uploads(tmp_path):
    db = _db(tmp_path)
    _add(db, "x-1", markdown="body")
    _add(db, "x-2", meta={"company_name": "Acme"})
    written = {}
    uploaded = {}

    def fake_writer(records, path):
        written["records"] = records
        written["path"] = path

    def fake_uploader(path, repo, token):
        uploaded.update(path=path, repo=repo, token=token)

    n = hf_sync.sync_exhibits(db, "arthrod/sec-ex10-exhibits", token="tok",
                              writer=fake_writer, uploader=fake_uploader, tmp_path=str(tmp_path / "out.parquet"))
    assert n == 2
    assert len(written["records"]) == 2
    assert {r["accession"] for r in written["records"]} == {"x-1", "x-2"}
    assert uploaded["repo"] == "arthrod/sec-ex10-exhibits" and uploaded["token"] == "tok"
    assert uploaded["path"] == written["path"]  # uploads exactly what was written


def test_to_records_coerces_non_string_fields_to_str():
    rows = [{"id": 1, "cik": 12345, "sequence": 2, "accession": "a", "markdown": "x"}]
    rec = hf_sync.to_records(rows)[0]
    assert rec["cik"] == "12345" and rec["sequence"] == "2"  # ints -> str for stable parquet schema
    assert isinstance(rec["accession"], str)


def test_sync_noop_on_empty_db(tmp_path):
    from sec_listener.db import Database
    db = Database(str(tmp_path / "e.db")); db.init()  # no rows
    calls = []
    n = hf_sync.sync_exhibits(db, "repo", token="t",
                              writer=lambda *a: calls.append("w"), uploader=lambda *a: calls.append("u"))
    assert n == 0 and calls == []  # empty -> no parquet write (pyarrow would raise), no upload
