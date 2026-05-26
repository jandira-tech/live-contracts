"""Tests for the exhibit HTML -> Markdown converter."""
import pytest

from sec_listener.converter import convert_html_to_markdown


def test_converts_basic_html_heading_and_paragraph():
    html = "<html><body><h1>Material Contract</h1><p>This Agreement is made.</p></body></html>"
    md = convert_html_to_markdown(html)
    assert "Material Contract" in md
    assert "This Agreement is made." in md
    # markdown heading marker should appear
    assert "#" in md


def test_strips_tags_to_plain_markdown_text():
    html = "<p>Hello <b>bold</b> and <i>italic</i> text.</p>"
    md = convert_html_to_markdown(html)
    assert "<b>" not in md and "<p>" not in md
    assert "bold" in md and "italic" in md


def test_empty_or_none_input_returns_empty_string():
    assert convert_html_to_markdown("") == ""
    assert convert_html_to_markdown(None) == ""


def test_malformed_html_does_not_raise():
    # Robustness: garbage in should not blow up the listener.
    md = convert_html_to_markdown("<<<not really >html<<< &nbsp; broken")
    assert isinstance(md, str)


def test_handles_table_content():
    html = "<table><tr><td>Party A</td><td>Party B</td></tr></table>"
    md = convert_html_to_markdown(html)
    assert "Party A" in md and "Party B" in md
