"""Pure parsing/classification helpers (no I/O) for the SEC listener."""
from __future__ import annotations

import logging
import math
import re
import xml.etree.ElementTree as ET

logger = logging.getLogger(__name__)

_ATOM = {"atom": "http://www.w3.org/2005/Atom"}
_ACCESSION_RE = re.compile(r"/(\d{10})-?(\d{2})-?(\d{6})")
_CIK_RE = re.compile(r"/data/(\d+)/")
_FILED_RE = re.compile(r"Filed:</b>\s*(\d{4}-\d{2}-\d{2})")
_SIZE_RE = re.compile(r"Size:</b>\s*([0-9.]+)\s*([A-Za-z]+)?")

# Binary unit multipliers (1 KB = 1024 bytes) — mirrors the Rust producer.
_SIZE_UNITS = {
    "B": 1,
    "BYTE": 1,
    "BYTES": 1,
    "KB": 1024,
    "K": 1024,
    "MB": 1024 * 1024,
    "M": 1024 * 1024,
    "GB": 1024 * 1024 * 1024,
    "G": 1024 * 1024 * 1024,
}


def filing_txt_url(cik: str, accession: str) -> str:
    """Build the modern/canonical EDGAR full-submission .txt URL.

    The path includes the accession-number folder (digits only), matching the
    Rust producer and the D1 schema::

        https://www.sec.gov/Archives/edgar/data/{cik}/{accession_nodash}/{accession}.txt
    """
    nodash = accession.replace("-", "")
    return f"https://www.sec.gov/Archives/edgar/data/{cik}/{nodash}/{accession}.txt"


def parse_summary_size_bytes(summary: str) -> int | None:
    """Parse the ``<b>Size:</b> N UNIT`` token from a SEC RSS summary into bytes.

    Mirrors the Rust producer's parser (1 KB = 1024 bytes). Returns ``None`` when
    no size marker is present or the value/unit is unrecognised.
    """
    m = _SIZE_RE.search(summary or "")
    if not m:
        return None
    try:
        value = float(m.group(1))
    except (TypeError, ValueError):
        return None
    # Reject NaN and ±inf (e.g. an absurdly long digit run overflows float to inf):
    # round() on a non-finite raises OverflowError/ValueError, so guard up front.
    if not math.isfinite(value) or value < 0:
        return None
    unit = (m.group(2) or "B").upper()
    multiplier = _SIZE_UNITS.get(unit)
    if multiplier is None:
        return None
    try:
        return round(value * multiplier)
    except (OverflowError, ValueError):
        return None


def parse_rss_feed(rss_content: str) -> list[dict]:
    """Parse a SEC getcurrent Atom feed into a list of filing dicts.

    Never raises: malformed XML or individual bad entries are skipped.
    """
    try:
        root = ET.fromstring(rss_content)
    except ET.ParseError as exc:
        logger.warning("RSS parse error: %s", exc)
        return []

    filings: list[dict] = []
    for entry in root.findall("atom:entry", _ATOM):
        try:
            link = entry.find("atom:link", _ATOM)
            url = link.get("href") if link is not None else None
            if not url:
                continue

            m = _ACCESSION_RE.search(url)
            if not m:
                continue
            accession = f"{m.group(1)}-{m.group(2)}-{m.group(3)}"

            cik_m = _CIK_RE.search(url)
            cik = cik_m.group(1) if cik_m else ""

            cat = entry.find("atom:category", _ATOM)
            form_type = cat.get("term", "") if cat is not None else ""

            summary_el = entry.find("atom:summary", _ATOM)
            summary_text = (summary_el.text if summary_el is not None else "") or ""
            filed_m = _FILED_RE.search(summary_text)
            filing_date = filed_m.group(1) if filed_m else ""

            filings.append(
                {
                    "accession": accession,
                    "cik": cik,
                    "form_type": form_type,
                    "filing_date": filing_date,
                    "url": url,
                    "summary": summary_text,
                    "size_bytes": parse_summary_size_bytes(summary_text),
                }
            )
        except Exception as exc:  # noqa: BLE001 - one bad entry must not stop the feed
            logger.warning("skipping bad RSS entry: %s", exc)
            continue

    return filings


def extract_filing_header(content: dict | None) -> dict:
    """Pull a compact filing header from datamule's parsed submission metadata.

    Input is ``Submission.metadata.content`` (datamule already parses the SGML
    <SEC-HEADER> for us — we just select a few useful fields). Never raises;
    always returns the full key set with empty defaults for missing data.
    """
    header = {
        "company_name": "",
        "cik": "",
        "sic": "",
        "state_of_incorporation": "",
        "period": "",
        "filing_date": "",
        "filed_at": "",
        "file_number": "",
        "location": "",
        "items": [],
    }
    # Malformed SGML can parse `content` itself — or any nested section — as a
    # non-dict (str/list/number); guard every level so a single bad filing never
    # raises into the listener loop.
    if not isinstance(content, dict):
        return header

    filer = content.get("filer")
    if isinstance(filer, list):
        filer = filer[0] if filer else {}
    if not isinstance(filer, dict):
        filer = {}
    company = filer.get("company-data")
    company = company if isinstance(company, dict) else {}
    values = filer.get("filing-values")
    values = values if isinstance(values, dict) else {}
    addr = filer.get("business-address")
    addr = addr if isinstance(addr, dict) else {}

    header["company_name"] = company.get("conformed-name", "") or ""
    header["cik"] = company.get("cik", "") or ""
    header["sic"] = company.get("assigned-sic", "") or ""
    header["state_of_incorporation"] = company.get("state-of-incorporation", "") or ""
    header["file_number"] = values.get("file-number", "") or ""
    header["period"] = content.get("period", "") or ""
    header["filing_date"] = content.get("filing-date", "") or ""
    # Actual SEC acceptance timestamp (ET), "YYYYMMDDHHMMSS" — the real filing time.
    header["filed_at"] = content.get("acceptance-datetime", "") or ""

    city, state = addr.get("city", ""), addr.get("state", "")
    header["location"] = ", ".join(p for p in (city, state) if p)

    items = content.get("item-information")
    if isinstance(items, str):
        header["items"] = [items]
    elif isinstance(items, list):
        header["items"] = list(items)

    return header


def classify_documents(documents: list[dict]) -> tuple[list[dict], list[dict]]:
    """Split documents into (traditional EX-10, other EX-* exhibits).

    Traditional EX-10 = ``EX-10`` or ``EX-10.N`` (material contracts).
    Excludes XBRL-style ``EX-101``/``EX-100`` etc. Non EX- documents are ignored.
    """
    ex10_docs: list[dict] = []
    other_ex_docs: list[dict] = []

    for doc in documents:
        doc_type = (doc.get("type") or "").strip()
        if doc_type.startswith("EX-10"):
            suffix = doc_type[5:]
            if suffix == "" or suffix.startswith("."):
                ex10_docs.append(doc)
            else:
                other_ex_docs.append(doc)
        elif doc_type.startswith("EX-"):
            other_ex_docs.append(doc)

    return ex10_docs, other_ex_docs
