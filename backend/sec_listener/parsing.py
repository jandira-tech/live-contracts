"""Pure parsing/classification helpers (no I/O) for the SEC listener."""
from __future__ import annotations

import logging
import re
import xml.etree.ElementTree as ET

logger = logging.getLogger(__name__)

_ATOM = {"atom": "http://www.w3.org/2005/Atom"}
_ACCESSION_RE = re.compile(r"/(\d{10})-?(\d{2})-?(\d{6})")
_CIK_RE = re.compile(r"/data/(\d+)/")
_FILED_RE = re.compile(r"Filed:</b>\s*(\d{4}-\d{2}-\d{2})")


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
        "file_number": "",
        "location": "",
        "items": [],
    }
    if not content:
        return header

    filer = content.get("filer")
    if isinstance(filer, list):
        filer = filer[0] if filer else {}
    filer = filer or {}
    company = filer.get("company-data") or {}
    values = filer.get("filing-values") or {}
    addr = filer.get("business-address") or {}

    header["company_name"] = company.get("conformed-name", "") or ""
    header["cik"] = company.get("cik", "") or ""
    header["sic"] = company.get("assigned-sic", "") or ""
    header["state_of_incorporation"] = company.get("state-of-incorporation", "") or ""
    header["file_number"] = values.get("file-number", "") or ""
    header["period"] = content.get("period", "") or ""
    header["filing_date"] = content.get("filing-date", "") or ""

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
