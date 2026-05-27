"""Push *finalized* rows from the local SQLite buffer to D1 via /api/ingest.
Write-once: a row is pushed only when complete. SQLite stays the working store;
D1 is authoritative. Failures leave rows unmirrored for retry; never raises."""
from __future__ import annotations
import json, logging
from typing import Any, Callable

logger = logging.getLogger(__name__)
_FIELDS = ["id","accession","cik","form_type","doc_type","filename","description","sequence",
           "filing_url","found_at","filed_at","markdown_status","filing_metadata","image_urls","markdown"]


def to_ingest_record(row: dict[str, Any]) -> dict[str, Any]:
    rec = {k: row.get(k) for k in _FIELDS}
    if not rec.get("filed_at"):
        meta = row.get("filing_metadata") or ""
        try: rec["filed_at"] = (json.loads(meta).get("filed_at") or "") if meta else ""
        except (ValueError, TypeError): rec["filed_at"] = ""
    return rec


def _http_poster(url: str, rows: list[dict[str, Any]], *, key: str) -> list[str]:
    import httpx
    resp = httpx.post(url, headers={"X-API-Key": key}, json={"rows": rows}, timeout=60)
    resp.raise_for_status()
    return resp.json().get("accepted", [])


def push_finalized(db, url: str, key: str, *, batch: int = 100,
                   poster: Callable[[str, list[dict]], list[str]] | None = None) -> int:
    rows = db.finalized_unmirrored(limit=batch)
    if not rows:
        return 0
    records = [to_ingest_record(r) for r in rows]
    send = poster or (lambda u, rs: _http_poster(u, rs, key=key))
    try:
        accepted = set(send(url, records))
    except Exception as exc:  # noqa: BLE001
        logger.warning("D1 push failed: %s", exc)
        return 0
    pushed = [r["id"] for r in rows if r["accession"] in accepted]
    db.mark_mirrored(pushed)
    return len(pushed)
