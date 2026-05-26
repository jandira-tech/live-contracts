"""Convert exhibit documents (HTML/PDF/text) to Markdown.

Uses MarkItDown (Microsoft) as the conversion engine — a solid, well-maintained
dependency that handles HTML, PDF, DOCX and plain text. We never hand-roll
markdown conversion ourselves.
"""
from __future__ import annotations

import io
import logging

from markitdown import MarkItDown

logger = logging.getLogger(__name__)

# A single reusable engine. MarkItDown is cheap to construct but reuse avoids
# repeated plugin discovery on hot paths (backfilling thousands of exhibits).
_ENGINE = MarkItDown(enable_plugins=False)


def convert_html_to_markdown(html: str | None, *, extension: str = ".html") -> str:
    """Convert an HTML (or other) document string to Markdown.

    Returns an empty string for empty/None input. Never raises: malformed input
    yields a best-effort string (possibly empty) so a single bad exhibit can
    never crash the listener.
    """
    if not html:
        return ""

    data = html.encode("utf-8", errors="replace") if isinstance(html, str) else html
    try:
        result = _ENGINE.convert_stream(io.BytesIO(data), file_extension=extension)
        return (result.text_content or "").strip()
    except Exception as exc:  # noqa: BLE001 - robustness is the whole point here
        logger.warning("markdown conversion failed (%s); returning empty", exc)
        return ""
