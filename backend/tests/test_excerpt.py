"""Tests for excerpt cleaning used in API list payloads."""
from sec_listener.api import clean_excerpt


def test_strips_markdown_emphasis_pipes_and_exhibit_label():
    md = "**Exhibit 10.1** TARGET NOTE | | | | --- | --- | | **Re:** stuff"
    out = clean_excerpt(md, limit=200)
    assert "**" not in out
    assert "|" not in out
    assert "TARGET NOTE" in out
    assert "Exhibit 10.1" not in out  # exhibit label is metadata noise, not content


def test_collapses_blank_lines_but_keeps_single_breaks():
    md = "Line one\n\n\n   Line two\t\tend"
    out = clean_excerpt(md, limit=200)
    assert "  " not in out          # no doubled spaces
    assert out == "Line one\nLine two end"  # blank-line run -> single break, kept


def test_strips_leading_image_ref_and_exhibit_label():
    md = '(exhibit101facilityagreem001.jpg "slide1") Exhibit 10.1 Execution Version FACILITY AGREEMENT DATED 19 MAY 2026'
    out = clean_excerpt(md, limit=200)
    assert ".jpg" not in out and "slide1" not in out
    assert "Exhibit 10.1" not in out
    assert out.lstrip().startswith("Execution Version FACILITY AGREEMENT")


def test_all_image_refs_and_label_collapses_to_empty():
    md = "Exhibit 10.3 (ex10-3_001.jpg) (ex10-3_002.jpg) (ex10-3_003.jpg) (ex10-3_004.jpg)"
    out = clean_excerpt(md, limit=200)
    assert ".jpg" not in out
    assert out.strip() == ""  # nothing but metadata -> empty preview, not garbage


def test_respects_limit_without_cutting_midword():
    md = "alpha beta gamma delta epsilon"
    out = clean_excerpt(md, limit=12)
    assert len(out) <= 13  # may include an ellipsis char
    assert "alph" in out
    assert out.endswith("…") or out in md


def test_empty_input():
    assert clean_excerpt("", limit=10) == ""
    assert clean_excerpt(None, limit=10) == ""
