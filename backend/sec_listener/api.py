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

import logging
import math
import os

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
    ):
        total = db.count_ex10()
        total_pages = max(1, math.ceil(total / page_size)) if total else 0
        offset = (page - 1) * page_size
        items = db.recent_ex10(limit=page_size, offset=offset)
        response.headers["Cache-Control"] = _LIST_CACHE
        return {
            "items": [_summary(i) for i in items],
            "total": total,
            "page": page,
            "page_size": page_size,
            "total_pages": total_pages,
        }

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

    @app.get("/api/ex10/{exhibit_id}", dependencies=[Depends(require_key)])
    def ex10_detail(exhibit_id: int, response: Response):
        with db.connect() as conn:
            row = conn.execute(
                "SELECT * FROM ex10_exhibits WHERE id = ?", (exhibit_id,)
            ).fetchone()
        if row is None:
            raise HTTPException(status_code=404, detail="exhibit not found")
        response.headers["Cache-Control"] = _DETAIL_CACHE
        return dict(row)

    return app


def _summary(row: dict) -> dict:
    """List payload: metadata + a short excerpt, not the full markdown body."""
    md = row.get("markdown") or ""
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
        "excerpt": md[:280],
        "has_markdown": bool(md),
    }


def main():
    import uvicorn

    logging.basicConfig(level=logging.INFO, format="%(asctime)s [%(levelname)s] %(name)s: %(message)s")
    config = Config.from_env()
    db = Database(config.db_path)
    db.init()
    app = create_app(db, api_key=os.environ.get("SEC_API_KEY"))
    host = os.environ.get("SEC_API_HOST", "127.0.0.1")  # localhost by default — not public
    port = int(os.environ.get("SEC_API_PORT", "8799"))
    logger.info("Internal API listening on %s:%d (key=%s)", host, port, "set" if os.environ.get("SEC_API_KEY") else "OPEN")
    uvicorn.run(app, host=host, port=port)


if __name__ == "__main__":
    main()
