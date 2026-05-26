"""Parallel Hugging Face dataset sink for the EX-10 store.

The local SQLite (``ex10_listener.db``) stays authoritative — this mirrors the
``ex10_exhibits`` table to a public HF dataset as a single Parquet snapshot,
queryable via DuckDB's ``hf://`` protocol:

    SELECT * FROM 'hf://datasets/arthrod/sec-ex10-exhibits/data/exhibits.parquet'

Guarded by ``HF_TOKEN``: with no token the sync is a no-op, so the SQLite store
remains the sole (plan-B) destination. Heavy deps (pyarrow / huggingface_hub)
are imported lazily inside the runtime writer/uploader so the pure logic and the
test suite need neither installed.
"""
from __future__ import annotations

import logging
import os
import tempfile
from typing import Any, Callable

logger = logging.getLogger(__name__)

DATASET_REPO = "arthrod/sec-ex10-exhibits"
PATH_IN_REPO = "data/exhibits.parquet"

# Columns mirrored from ex10_exhibits (full markdown included; parquet compresses).
_COLUMNS = [
    "id", "accession", "cik", "form_type", "doc_type", "filename", "description",
    "sequence", "filing_url", "found_at", "markdown_status", "filing_metadata", "markdown",
]
_STR_FIELDS = [c for c in _COLUMNS if c not in ("id", "markdown")]


def fetch_all_exhibits(db) -> list[dict[str, Any]]:
    """Read every ex10_exhibits row (id-ordered). Tolerates legacy DBs missing newer columns."""
    cols = set(db.column_names("ex10_exhibits"))
    select = ", ".join(c for c in _COLUMNS if c in cols)
    with db.connect() as conn:
        rows = conn.execute(f"SELECT {select} FROM ex10_exhibits ORDER BY id").fetchall()
    return [dict(r) for r in rows]


def to_records(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Normalize rows into a stable, JSON/parquet-safe schema.

    Adds ``has_markdown``; coerces missing string fields to "" (never None) so the
    Parquet column types stay consistent across snapshots.
    """
    records = []
    for r in rows:
        md = r.get("markdown") or ""
        rec: dict[str, Any] = {"id": r.get("id")}
        for f in _STR_FIELDS:
            rec[f] = r.get(f) or ""
        rec["has_markdown"] = bool(md)
        rec["markdown"] = md
        records.append(rec)
    return records


def write_parquet(records: list[dict[str, Any]], path: str) -> None:
    """Runtime writer: serialize records to a Parquet file (pyarrow)."""
    import pyarrow as pa
    import pyarrow.parquet as pq

    pq.write_table(pa.Table.from_pylist(records), path)


def upload_parquet(path: str, repo: str, token: str, path_in_repo: str = PATH_IN_REPO) -> None:
    """Runtime uploader: commit the Parquet snapshot to the HF dataset."""
    from huggingface_hub import HfApi

    HfApi(token=token).upload_file(
        path_or_fileobj=path,
        path_in_repo=path_in_repo,
        repo_id=repo,
        repo_type="dataset",
        commit_message="sync ex10_exhibits snapshot",
    )


def sync_exhibits(
    db,
    repo: str = DATASET_REPO,
    *,
    token: str | None = None,
    writer: Callable[[list[dict[str, Any]], str], None] = write_parquet,
    uploader: Callable[[str, str, str], None] = upload_parquet,
    tmp_path: str | None = None,
) -> int:
    """Snapshot all exhibits to the HF dataset; return the row count synced.

    No-op (returns 0) when ``token`` is falsy — SQLite stays the sole store.
    """
    if not token:
        logger.debug("HF dataset sync skipped (no token)")
        return 0
    records = to_records(fetch_all_exhibits(db))
    path = tmp_path or os.path.join(tempfile.mkdtemp(prefix="ex10-sync-"), "exhibits.parquet")
    writer(records, path)
    uploader(path, repo, token)
    logger.info("Synced %d exhibits to HF dataset %s", len(records), repo)
    return len(records)
