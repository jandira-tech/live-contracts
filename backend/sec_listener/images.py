"""Capture scanned-exhibit images and host them in the HF dataset.

Some EX-10 exhibits are scanned documents whose HTML is just ``<IMG SRC=...>``
wrappers — markitdown yields only ``(foo.jpg)`` refs, so the text preview is
(correctly) empty. To still surface the content we pull the ``GRAPHIC`` documents
from the SEC filing and store them in the public HF dataset
([[sec-listener-hf-dataset-sink]]) under ``images/{accession}/…``; their public
``resolve/main`` URLs go in an ``image_urls`` column and render as a gallery.

HF_TOKEN-guarded like the dataset sink: no token -> no-op. datamule/HF I/O is
injected so the pure logic is testable without network.
"""
from __future__ import annotations

import logging
import re
from typing import Callable

logger = logging.getLogger(__name__)

DATASET_REPO = "arthrod/sec-ex10-exhibits"
# Capture the referenced filename from a markdown image ref: (ex10-3_001.jpg) or
# (exhibit101.jpg "slide1").
_IMG_REF = re.compile(r"\(\s*([^()\s\"]+\.(?:jpe?g|png|gif|tiff?|svg|webp))(?:\s+\"[^\"]*\")?\s*\)", re.I)


def image_filenames(markdown: str | None) -> list[str]:
    """Image filenames referenced in the exhibit markdown — in order, deduped."""
    if not markdown:
        return []
    seen: set[str] = set()
    out: list[str] = []
    for m in _IMG_REF.finditer(markdown):
        fn = m.group(1).strip()
        if fn and fn not in seen:
            seen.add(fn)
            out.append(fn)
    return out


def is_image_only(markdown: str | None, *, clean_excerpt_fn: Callable[[str], str]) -> bool:
    """True when the body is essentially just image refs — images present and no
    readable text survives ``clean_excerpt`` (which strips image refs + labels)."""
    if not markdown or not image_filenames(markdown):
        return False
    return clean_excerpt_fn(markdown).strip() == ""


def dataset_image_path(accession: str, filename: str) -> str:
    return f"images/{accession}/{filename}"


def public_url(repo: str, path_in_repo: str) -> str:
    return f"https://huggingface.co/datasets/{repo}/resolve/main/{path_in_repo}"


def fetch_graphics(accession: str, cik: str) -> list[tuple[str, bytes]]:
    """Pull GRAPHIC (image) documents for a filing as (filename, bytes) pairs."""
    from datamule import Submission, format_accession

    formatted = format_accession(accession.replace("-", ""), "dash")
    url = f"https://www.sec.gov/Archives/edgar/data/{cik}/{formatted}.txt"
    sub = Submission(url=url)
    out: list[tuple[str, bytes]] = []
    for d in sub.document_type(["GRAPHIC"]):
        data = d.content
        if isinstance(data, str):
            data = data.encode("latin-1", errors="ignore")
        if d.filename and data:
            out.append((d.filename, data))
    return out


def upload_images(uploads: list[tuple[bytes, str]], repo: str, token: str) -> None:
    """Upload all of a filing's images in ONE commit (not one per image)."""
    from huggingface_hub import CommitOperationAdd, HfApi

    ops = [CommitOperationAdd(path_in_repo=path, path_or_fileobj=data) for data, path in uploads]
    HfApi(token=token).create_commit(
        repo_id=repo,
        repo_type="dataset",
        operations=ops,
        commit_message=f"add {len(ops)} exhibit image(s)",
    )


def capture_images(
    accession: str,
    cik: str,
    *,
    token: str | None,
    repo: str = DATASET_REPO,
    fetcher: Callable[[str, str], list[tuple[str, bytes]]] = fetch_graphics,
    uploader: Callable[[list[tuple[bytes, str]], str, str], None] = upload_images,
) -> list[str]:
    """Fetch a filing's images, store them in the dataset (one commit), return URLs.

    No-op (returns []) without a token. Network failures are logged and swallowed
    so a transient SEC/HF hiccup never crashes the caller.
    """
    if not token:
        return []
    try:
        graphics = fetcher(accession, cik)
        if not graphics:
            return []
        uploads: list[tuple[bytes, str]] = []
        urls: list[str] = []
        for filename, data in graphics:
            path = dataset_image_path(accession, filename)
            uploads.append((data, path))
            urls.append(public_url(repo, path))
        uploader(uploads, repo, token)
        return urls
    except Exception as exc:  # noqa: BLE001 - transient SEC/HF errors must not crash the worker
        logger.warning("image capture failed for %s: %s", accession, exc)
        return []
