"""Tests for excerpt cleaning used in API list payloads."""
from sec_listener.api import clean_excerpt


def test_strips_markdown_emphasis_and_pipes():
    md = "**Exhibit 10.1** TARGET NOTE | | | | --- | --- | | **Re:** stuff"
    out = clean_excerpt(md, limit=200)
    assert "**" not in out
    assert "|" not in out
    assert "TARGET NOTE" in out
    assert "Exhibit 10.1" in out


def test_collapses_whitespace_and_trims():
    md = "Line one\n\n\n   Line two\t\tend"
    out = clean_excerpt(md, limit=200)
    assert "  " not in out
    assert out == "Line one Line two end"


def test_respects_limit_without_cutting_midword():
    md = "alpha beta gamma delta epsilon"
    out = clean_excerpt(md, limit=12)
    assert len(out) <= 13  # may include an ellipsis char
    assert "alph" in out
    # never ends mid a partial word with no marker
    assert out.endswith("…") or out in md


def test_empty_input():
    assert clean_excerpt("", limit=10) == ""
    assert clean_excerpt(None, limit=10) == ""
