"""Internal read-only API over the EX-10 exhibit store.

This API is **internal only**. It is bound to localhost by default and gated by
an API key (``SEC_API_KEY``). The public-facing surface is the Cloudflare Worker
+ Astro frontend, which calls this origin over a private channel (Cloudflare
Tunnel / WARP / private network) carrying the key. Do not expose this port to
the internet.

Responses carry ``Cache-Control`` with ``stale-while-revalidate`` so the CDN
edge can serve cached data instantly while revalidating in the background.
"""
from __future__ import annotations

import json
import logging
import math
import re

from fastapi import Depends, FastAPI, Header, HTTPException, Query, Response

from .config import Config
from .db import Database

logger = logging.getLogger(__name__)

# Light list payloads cache a little longer than the 60s refresh; the "since"
# feed is fresh-ish but SWR lets the edge serve instantly while revalidating.
_LIST_CACHE = "public, max-age=30, stale-while-revalidate=60"
_SINCE_CACHE = "public, max-age=10, stale-while-revalidate=60"
_DETAIL_CACHE = "public, max-age=300, stale-while-revalidate=600"


def create_app(db: Database, api_key: str | None = None) -> FastAPI:
    app = FastAPI(title="SEC EX-10 Internal API", version="0.2.0", docs_url=None, redoc_url=None)

    def require_key(x_api_key: str | None = Header(default=None)) -> None:
        if api_key and x_api_key != api_key:
            raise HTTPException(status_code=401, detail="invalid or missing API key")

    @app.get("/health")
    def health():
        return {"status": "ok", "total_ex10": db.count_ex10()}

    @app.get("/api/ex10", dependencies=[Depends(require_key)])
    def list_ex10(
        response: Response,
        page: int = Query(1, ge=1),
        page_size: int = Query(20, ge=1, le=200),
        form: str | None = Query(None, max_length=40),
        cik: str | None = Query(None, max_length=20),
        filer: str | None = Query(None, max_length=120),
        sort: str = Query("newest"),
    ):
        oldest = sort == "oldest"
        total = db.count_ex10(form=form, cik=cik, filer=filer)
        total_pages = max(1, math.ceil(total / page_size)) if total else 0
        offset = (page - 1) * page_size
        items = db.recent_ex10(limit=page_size, offset=offset, form=form, cik=cik, filer=filer, oldest=oldest)
        response.headers["Cache-Control"] = _LIST_CACHE
        return {
            "items": [_summary(i) for i in items],
            "total": total,
            "page": page,
            "page_size": page_size,
            "total_pages": total_pages,
        }

    @app.get("/api/facets", dependencies=[Depends(require_key)])
    def facets(response: Response):
        response.headers["Cache-Control"] = _LIST_CACHE
        return {"forms": db.form_facets()}

    @app.get("/api/ex10/since", dependencies=[Depends(require_key)])
    def ex10_since(
        response: Response,
        seconds: int = Query(60, ge=1, le=86400),
    ):
        items = db.ex10_since(seconds=seconds)
        response.headers["Cache-Control"] = _SINCE_CACHE
        return {
            "window_seconds": seconds,
            "count": len(items),
            "items": [_summary(i) for i in items],
        }

    @app.get("/api/search", dependencies=[Depends(require_key)])
    def search(
        response: Response,
        q: str = Query("", max_length=200),
        page: int = Query(1, ge=1),
        page_size: int = Query(20, ge=1, le=200),
    ):
        response.headers["Cache-Control"] = _LIST_CACHE
        query = (q or "").strip()
        if not query:
            return {"query": "", "total": 0, "page": page, "page_size": page_size,
                    "total_pages": 0, "items": []}
        total = db.search_count(query)
        total_pages = max(1, math.ceil(total / page_size)) if total else 0
        items = db.search(query, limit=page_size, offset=(page - 1) * page_size)
        return {
            "query": query, "total": total, "page": page, "page_size": page_size,
            "total_pages": total_pages, "items": [_summary(i) for i in items],
        }

    @app.get("/api/stats", dependencies=[Depends(require_key)])
    def stats(response: Response):
        response.headers["Cache-Control"] = _LIST_CACHE
        return db.stats()

    @app.get("/api/ex10/{exhibit_id}", dependencies=[Depends(require_key)])
    def ex10_detail(exhibit_id: int, response: Response):
        with db.connect() as conn:
            row = conn.execute(
                "SELECT * FROM ex10_exhibits WHERE id = ?", (exhibit_id,)
            ).fetchone()
        if row is None:
            raise HTTPException(status_code=404, detail="exhibit not found")
        response.headers["Cache-Control"] = _DETAIL_CACHE
        out = dict(row)
        out["filing"] = _parse_filing(out.get("filing_metadata"))
        out["image_urls"] = _parse_image_urls(out.get("image_urls"))
        return out

    return app


_MD_MARKERS = re.compile(r"\*+|_{2,}|`+|#+|>+|\[|\]|!\[")
# Image references markitdown emits for scanned exhibits, e.g.
# `(ex10-3_001.jpg)` or `(exhibit101facilityagreem001.jpg "slide1")`.
_IMG_REF = re.compile(r"\([^()]*\.(?:jpe?g|png|gif|tiff?|svg|webp)(?:\s+\"[^\"]*\")?[^()]*\)", re.I)
# Leading exhibit label(s) — pure metadata at the START ("Exhibit 10.1", "EX-10.3").
# Anchored so mid-body references ("...subject to Exhibit 10.2...") are preserved.
_LEADING_LABEL = re.compile(r"^\s*(?:(?:exhibit|ex)[\s.\-]*\d+(?:\.\d+)?\b\s*)+", re.I)


def clean_excerpt(text: str | None, limit: int = 280) -> str:
    """Turn raw exhibit markdown into a clean preview.

    Drops scanned-image references and "Exhibit 10.x" labels (metadata, not
    contract text), strips markdown/table markers, and collapses whitespace
    while **preserving single line breaks** (blank-line runs become one break)
    so previews read as text instead of a grumbled run-on. Truncates on a word
    or line boundary with an ellipsis.
    """
    if not text:
        return ""
    s = text.replace("|", " ")
    s = _IMG_REF.sub(" ", s)                   # image refs are noise anywhere
    s = _MD_MARKERS.sub("", s)
    s = re.sub(r"-{2,}", " ", s)
    s = re.sub(r"[ \t]+", " ", s)              # collapse horizontal whitespace
    s = re.sub(r"[ \t]*\n[ \t]*", "\n", s)     # trim around newlines
    s = re.sub(r"\n{2,}", "\n", s).strip()     # blank-line runs -> single break
    s = _LEADING_LABEL.sub("", s).strip()      # strip only LEADING exhibit label(s)
    if len(s) <= limit:
        return s
    cut = s[:limit]
    boundary = max(cut.rfind(" "), cut.rfind("\n"))
    if boundary > 0:
        cut = cut[:boundary]
    return cut.rstrip() + "…"


def _parse_filing(raw: str | None) -> dict:
    if not raw:
        return {}
    try:
        val = json.loads(raw)
    except (ValueError, TypeError):
        return {}
    # Valid JSON that isn't an object (list/str/number) would break downstream .get().
    return val if isinstance(val, dict) else {}


def _parse_image_urls(raw: str | None) -> list[str]:
    if not raw:
        return []
    try:
        val = json.loads(raw)
    except (ValueError, TypeError):
        return []
    return [str(u) for u in val] if isinstance(val, list) else []


def _summary(row: dict) -> dict:
    """List payload: metadata + a short excerpt, not the full markdown body."""
    md = row.get("markdown") or ""
    filing = _parse_filing(row.get("filing_metadata"))
    return {
        "id": row.get("id"),
        "accession": row.get("accession"),
        "cik": row.get("cik"),
        "form_type": row.get("form_type"),
        "doc_type": row.get("doc_type"),
        "filename": row.get("filename"),
        "description": row.get("description"),
        "filing_url": row.get("filing_url"),
        "found_at": row.get("found_at"),
        "markdown_status": row.get("markdown_status"),
        "excerpt": clean_excerpt(md, 520),  # enough to fill ~4 lines; CSS clamps
        "has_markdown": bool(md),
        # Compact filing header for card footers (company, period of report, items).
        "company_name": filing.get("company_name", ""),
        "period": filing.get("period", ""),
        "location": filing.get("location", ""),
        "items": filing.get("items", []),
        # Actual SEC acceptance time (ET, "YYYYMMDDHHMMSS"); "" until backfilled.
        "filed_at": filing.get("filed_at", ""),
        # Public HF-dataset URLs for scanned (image-only) exhibits; [] otherwise.
        "image_urls": _parse_image_urls(row.get("image_urls")),
    }


def make_api_server(config: Config, db: Database):
    """Build a uvicorn.Server for the internal API, bound per Config.

    Returned (not started) so it can be awaited as a task alongside the worker's
    other loops, or run standalone.
    """
    import uvicorn

    app = create_app(db, api_key=config.api_key)
    uconfig = uvicorn.Config(
        app,
        host=config.api_host,  # localhost by default — not public
        port=config.api_port,
        log_level="warning",
    )
    return uvicorn.Server(uconfig)


def main():
    logging.basicConfig(level=logging.INFO, format="%(asctime)s [%(levelname)s] %(name)s: %(message)s")
    config = Config.from_env()
    db = Database(config.db_path)
    db.init()
    server = make_api_server(config, db)
    logger.info(
        "Internal API listening on %s:%d (key=%s)",
        config.api_host, config.api_port, "set" if config.api_key else "OPEN",
    )
    server.run()


if __name__ == "__main__":
    main()
