"""Tests for extracting a compact filing header from datamule's parsed metadata."""
from sec_listener.parsing import extract_filing_header

# Exactly the structure datamule's Submission.metadata.content produces.
SAMPLE = {
    "acceptance-datetime": "20260526113637",
    "accession-number": "0001679273-26-000018",
    "type": "8-K",
    "public-document-count": "102",
    "period": "20260519",
    "item-information": [
        "Entry into a Material Definitive Agreement",
        "Termination of a Material Definitive Agreement",
        "Financial Statements and Exhibits",
    ],
    "filing-date": "20260526",
    "filer": {
        "company-data": {
            "conformed-name": "Lamb Weston Holdings, Inc.",
            "cik": "0001679273",
            "assigned-sic": "2030",
            "state-of-incorporation": "DE",
        },
        "filing-values": {"form-type": "8-K", "file-number": "001-37830"},
        "business-address": {"city": "EAGLE", "state": "ID"},
    },
}


def test_extracts_compact_header_fields():
    h = extract_filing_header(SAMPLE)
    assert h["company_name"] == "Lamb Weston Holdings, Inc."
    assert h["cik"] == "0001679273"
    assert h["sic"] == "2030"
    assert h["state_of_incorporation"] == "DE"
    assert h["period"] == "20260519"
    assert h["filing_date"] == "20260526"
    assert h["file_number"] == "001-37830"
    assert h["location"] == "EAGLE, ID"
    assert h["items"] == SAMPLE["item-information"]


def test_filer_as_list_takes_first():
    content = {**SAMPLE, "filer": [SAMPLE["filer"], {"company-data": {"conformed-name": "Other"}}]}
    h = extract_filing_header(content)
    assert h["company_name"] == "Lamb Weston Holdings, Inc."


def test_item_information_as_single_string():
    content = {**SAMPLE, "item-information": "Financial Statements and Exhibits"}
    h = extract_filing_header(content)
    assert h["items"] == ["Financial Statements and Exhibits"]


def test_missing_fields_are_safe():
    h = extract_filing_header({})
    assert h["company_name"] == ""
    assert h["items"] == []
    assert h["location"] == ""
    # never raises, always returns the full key set
    assert set(h) >= {"company_name", "cik", "sic", "state_of_incorporation",
                      "period", "filing_date", "file_number", "location", "items"}


def test_none_input_returns_empty_header():
    h = extract_filing_header(None)
    assert h["company_name"] == "" and h["items"] == []


def test_api_surfaces_filing_fields(tmp_path):
    from fastapi.testclient import TestClient
    from sec_listener.api import create_app
    from sec_listener.db import Database

    db = Database(str(tmp_path / "f.db"))
    db.init()
    db.save_ex10_exhibit(
        {"accession": "f-1", "cik": "1", "form_type": "8-K", "doc_type": "EX-10.1",
         "filename": "a.htm", "description": "Deal", "sequence": "1", "url": "u"},
        markdown="body",
        filing_metadata={"company_name": "Lamb Weston Holdings, Inc.", "period": "20260519",
                         "location": "EAGLE, ID", "items": ["Entry into a Material Definitive Agreement"]},
    )
    client = TestClient(create_app(db, api_key=None))
    # Card payload exposes compact fields
    item = client.get("/api/ex10").json()["items"][0]
    assert item["company_name"] == "Lamb Weston Holdings, Inc."
    assert item["period"] == "20260519"
    assert item["location"] == "EAGLE, ID"
    # Detail exposes the full parsed filing object
    detail = client.get(f"/api/ex10/{item['id']}").json()
    assert detail["filing"]["company_name"] == "Lamb Weston Holdings, Inc."
    assert detail["filing"]["items"] == ["Entry into a Material Definitive Agreement"]


def test_extract_filing_header_tolerates_non_dict_nested():
    """Malformed SGML may parse nested sections as non-dicts; must not raise."""
    from sec_listener.parsing import extract_filing_header
    bad = {
        "filer": {"company-data": "GARBLED", "filing-values": ["x"], "business-address": 7},
        "period": "20260519",
    }
    out = extract_filing_header(bad)
    assert out["company_name"] == ""
    assert out["file_number"] == ""
    assert out["location"] == ""
    assert out["period"] == "20260519"
    # filer itself as a bare string must also be tolerated
    assert extract_filing_header({"filer": "nope"})["company_name"] == ""


def test_extract_filing_header_tolerates_non_dict_content():
    """content itself may parse as a non-dict (list/str) on severely malformed SGML."""
    from sec_listener.parsing import extract_filing_header
    assert extract_filing_header([1, 2])["company_name"] == ""
    assert extract_filing_header("garbage")["items"] == []
    assert extract_filing_header(42)["period"] == ""


def test_extract_filing_header_captures_acceptance_datetime():
    """The real filing time (SEC acceptance-datetime, ET) is surfaced as filed_at."""
    from sec_listener.parsing import extract_filing_header
    out = extract_filing_header({"acceptance-datetime": "20260526113637", "filing-date": "20260526"})
    assert out["filed_at"] == "20260526113637"
    # absent -> empty, never raises
    assert extract_filing_header({})["filed_at"] == ""
