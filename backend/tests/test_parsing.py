"""Tests for pure RSS parsing and EX-10 classification logic."""
import pytest

from sec_listener.parsing import (
    classify_documents,
    filing_txt_url,
    parse_rss_feed,
    parse_summary_size_bytes,
)


def test_filing_txt_url_uses_modern_accession_folder():
    # Modern/canonical EDGAR path includes the accession-number folder (no dashes),
    # matching the Rust producer and the D1 schema.
    url = filing_txt_url("719413", "0001437749-26-018228")
    assert url == (
        "https://www.sec.gov/Archives/edgar/data/719413/"
        "000143774926018228/0001437749-26-018228.txt"
    )

SAMPLE_FEED = """<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <entry>
    <title>8-K - EXAMPLE CORP</title>
    <link rel="alternate" type="text/html"
          href="https://www.sec.gov/Archives/edgar/data/719413/000143774926018228-index.htm"/>
    <category term="8-K"/>
    <summary type="html">&lt;b&gt;Filed:&lt;/b&gt; 2026-05-25 &lt;b&gt;AccNo:&lt;/b&gt; 0001437749-26-018228</summary>
  </entry>
  <entry>
    <title>10-K - NO ACCESSION CORP</title>
    <link rel="alternate" type="text/html" href="https://www.sec.gov/cgi-bin/no-accession"/>
    <category term="10-K"/>
    <summary type="html">no accession here</summary>
  </entry>
</feed>"""


def test_parse_rss_feed_extracts_accession_cik_form():
    filings = parse_rss_feed(SAMPLE_FEED)
    assert len(filings) == 1  # the second entry has no parseable accession
    f = filings[0]
    assert f["accession"] == "0001437749-26-018228"
    assert f["cik"] == "719413"
    assert f["form_type"] == "8-K"
    assert f["filing_date"] == "2026-05-25"


def test_parse_rss_feed_handles_empty_feed():
    empty = '<feed xmlns="http://www.w3.org/2005/Atom"></feed>'
    assert parse_rss_feed(empty) == []


def test_parse_rss_feed_returns_empty_on_garbage():
    # Robustness: invalid XML must not raise.
    assert parse_rss_feed("not xml at all <<<") == []


@pytest.mark.parametrize(
    "summary, expected",
    [
        ("<b>Filed:</b> 2026-05-22 <b>Size:</b> 4 KB", 4 * 1024),
        ("<b>Filed:</b> 2026-05-22 <b>Size:</b> 21 MB", 21 * 1024 * 1024),
        ("<b>Filed:</b> 2026-05-22 <b>Size:</b> 2 GB", 2 * 1024 * 1024 * 1024),
        ("<b>Filed:</b> 2026-05-22 <b>Size:</b> 512 B", 512),
        ("no size here", None),
    ],
)
def test_parse_summary_size_bytes(summary, expected):
    # Mirrors the Rust producer's parse_summary_size_bytes (1 KB = 1024 bytes).
    assert parse_summary_size_bytes(summary) == expected


def test_parse_rss_feed_carries_size_bytes():
    feed = (
        '<feed xmlns="http://www.w3.org/2005/Atom">'
        "<entry>"
        "<title>8-K - EXAMPLE CORP</title>"
        '<link rel="alternate" type="text/html" '
        'href="https://www.sec.gov/Archives/edgar/data/719413/'
        '000143774926018228-index.htm"/>'
        '<category term="8-K"/>'
        '<summary type="html">&lt;b&gt;Filed:&lt;/b&gt; 2026-05-25 '
        "&lt;b&gt;AccNo:&lt;/b&gt; 0001437749-26-018228 "
        "&lt;b&gt;Size:&lt;/b&gt; 21 MB&lt;/summary&gt;</summary>"
        "</entry>"
        "</feed>"
    )
    f = parse_rss_feed(feed)[0]
    assert f["size_bytes"] == 21 * 1024 * 1024


def test_parse_rss_feed_size_bytes_none_when_absent():
    f = parse_rss_feed(SAMPLE_FEED)[0]  # SAMPLE_FEED has no Size: marker
    assert f["size_bytes"] is None


def test_classify_documents_separates_traditional_ex10_from_xbrl():
    docs = [
        {"type": "EX-10.1", "filename": "a.htm"},
        {"type": "EX-10", "filename": "b.htm"},
        {"type": "EX-101.INS", "filename": "xbrl.xml"},  # XBRL, not material contract
        {"type": "EX-99.1", "filename": "press.htm"},
        {"type": "GRAPHIC", "filename": "img.jpg"},
    ]
    ex10, other = classify_documents(docs)
    ex10_types = {d["type"] for d in ex10}
    assert ex10_types == {"EX-10.1", "EX-10"}
    other_types = {d["type"] for d in other}
    assert "EX-101.INS" in other_types
    assert "EX-99.1" in other_types
    assert "GRAPHIC" not in other_types  # only EX- prefixed go to "other"
