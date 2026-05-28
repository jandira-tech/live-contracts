"""Background worker: continuous listening + Markdown backfill.

The worker is an **internal** process — it talks to SEC and the local SQLite DB
only. It is never network-exposed. It runs two concurrent jobs:

1. the hardened :class:`~sec_listener.listener.Listener` polling loop, and
2. a backfill loop that converts older EX-10 exhibits (and any the listener
   stored without markdown) to Markdown in batches.

Backfill is status-driven (``markdown_status``): ``done``/``empty``/``error`` are
terminal so we never hammer SEC for the same document forever.
"""
from __future__ import annotations

import asyncio
import logging
import os
import signal
import time

from .config import Config
from .converter import convert_html_to_markdown
from .db import Database
from .listener import Listener

logger = logging.getLogger(__name__)


def _as_text(content) -> str:
    if content is None:
        return ""
    if isinstance(content, (bytes, bytearray)):
        return content.decode("utf-8", errors="replace")
    return str(content)


class BackfillWorker:
    def __init__(self, db: Database, *, fetcher=None, convert_fn=convert_html_to_markdown,
                 metadata_fetcher=None, request_delay: float = 0.12, sleep_fn=time.sleep,
                 image_token: str | None = None, image_repo: str | None = None, image_capture_fn=None,
                 image_max_attempts: int = 5):
        self.db = db
        self.image_max_attempts = image_max_attempts
        # fetcher(accession, cik, filename) -> raw document content (str/bytes) or ""
        self.fetcher = fetcher or self._datamule_fetcher
        # metadata_fetcher(accession, cik) -> compact filing header dict
        self.metadata_fetcher = metadata_fetcher or self._datamule_metadata_fetcher
        self.convert_fn = convert_fn
        # Per-request throttle for the sec.gov backfill loops (markdown + metadata +
        # images): SEC caps clients at 10 req/s. Injectable so tests don't sleep.
        self.request_delay = request_delay
        self._sleep = sleep_fn
        # Scanned-exhibit image capture -> HF dataset. Opt-in via image_token (HF_TOKEN).
        self.image_token = image_token
        from .images import DATASET_REPO as _IMG_REPO
        self.image_repo = image_repo or _IMG_REPO
        self._capture_images = image_capture_fn or self._datamule_capture_images

    def backfill_batch(self, limit: int = 25) -> int:
        """Convert up to ``limit`` pending exhibits. Returns the number converted."""
        rows = self.db.exhibits_missing_markdown(limit=limit)
        converted = 0
        for row in rows:
            if self.request_delay > 0:
                self._sleep(self.request_delay)  # respect SEC's 10 req/s cap
            try:
                content = self.fetcher(row["accession"], row["cik"], row["filename"])
            except Exception as exc:  # noqa: BLE001 - isolate each row
                logger.warning("backfill fetch failed for %s: %s", row["accession"], exc)
                self.db.update_markdown(row["id"], "", status="error")
                continue

            text = _as_text(content)
            if not text:
                self.db.update_markdown(row["id"], "", status="empty")
                continue

            try:
                markdown = self.convert_fn(text)
            except Exception as exc:  # noqa: BLE001
                logger.warning("backfill convert failed for %s: %s", row["accession"], exc)
                self.db.update_markdown(row["id"], "", status="error")
                continue

            if markdown:
                self.db.update_markdown(row["id"], markdown, status="done")
                converted += 1
            else:
                self.db.update_markdown(row["id"], "", status="empty")
        return converted

    def backfill_metadata_batch(self, limit: int = 25) -> int:
        """Fill the filing-header metadata for up to ``limit`` exhibits missing it."""
        rows = self.db.exhibits_missing_filing_metadata(limit=limit)
        filled = 0
        for row in rows:
            if self.request_delay > 0:
                self._sleep(self.request_delay)  # respect SEC's 10 req/s cap
            try:
                meta = self.metadata_fetcher(row["accession"], row["cik"])
            except Exception as exc:  # noqa: BLE001 - isolate each row
                logger.warning("metadata fetch failed for %s: %s", row["accession"], exc)
                # Store empty so we don't re-hammer a broken filing forever.
                self.db.update_filing_metadata(row["id"], {})
                continue
            self.db.update_filing_metadata(row["id"], meta or {})
            if meta:
                filled += 1
        return filled

    def backfill_images_batch(self, limit: int = 25) -> int:
        """Capture images for scanned (image-only) exhibits → HF dataset URLs.

        Each converted exhibit is checked once: image-only rows get their image
        URLs, others are marked '[]' (so they're not rescanned). No-op without a
        token. Returns the number of rows for which images were captured.
        """
        if not self.image_token:
            return 0
        from .api import clean_excerpt
        from .images import image_filenames, is_image_only

        rows = self.db.exhibits_pending_images(limit=limit)
        captured = 0
        for row in rows:
            if not is_image_only(row["markdown"], clean_excerpt_fn=clean_excerpt):
                self.db.update_image_urls(row["id"], [])  # checked, not image-only
                continue
            if self.request_delay > 0:
                self._sleep(self.request_delay)  # respect SEC's 10 req/s cap
            only = set(image_filenames(row["markdown"]))  # only THIS exhibit's images
            try:
                urls = self._capture_images(row["accession"], row["cik"], only)
            except Exception as exc:  # noqa: BLE001 - isolate each row
                logger.warning("image capture failed for %s: %s", row["accession"], exc)
                urls = []
            if urls:
                self.db.update_image_urls(row["id"], urls)
                captured += 1
            else:
                if self.db.bump_image_attempts(row["id"]) >= self.image_max_attempts:
                    self.db.update_image_urls(row["id"], [])   # give up -> finalizes
        return captured

    # --- live fetch ---------------------------------------------------------
    def _datamule_capture_images(self, accession: str, cik: str, only: set[str] | None = None) -> list[str]:
        from .images import capture_images

        return capture_images(accession, cik, token=self.image_token, repo=self.image_repo, only=only)

    def _datamule_metadata_fetcher(self, accession: str, cik: str) -> dict:
        from datamule import Submission, format_accession

        from .parsing import extract_filing_header

        formatted = format_accession(accession.replace("-", ""), "dash")
        url = f"https://www.sec.gov/Archives/edgar/data/{cik}/{formatted}.txt"
        sub = Submission(url=url)
        return extract_filing_header(sub.metadata.content)

    def _datamule_fetcher(self, accession: str, cik: str, filename: str) -> str:
        from datamule import Submission, format_accession

        formatted = format_accession(accession.replace("-", ""), "dash")
        url = f"https://www.sec.gov/Archives/edgar/data/{cik}/{formatted}.txt"
        sub = Submission(url=url)
        docs = sub.metadata.content.get("documents", [])
        target = next((d for d in docs if d.get("filename") == filename), None)
        if target is None:
            return ""
        for d in sub.document_type([target.get("type")]):
            if d.filename == filename:
                return _as_text(d.content)
        return ""

    async def run(self, *, batch: int = 25, interval: float = 5.0, rps: float = 5.0):
        """Continuously drain the backfill queue, batch by batch."""
        logger.info("Backfill worker started (batch=%d, interval=%.1fs)", batch, interval)
        while True:
            try:
                n = await asyncio.to_thread(self.backfill_batch, batch)
                if n:
                    logger.info("Backfilled %d exhibits to markdown", n)
                m = await asyncio.to_thread(self.backfill_metadata_batch, batch)
                if m:
                    logger.info("Backfilled %d exhibits with filing metadata", m)
                img = await asyncio.to_thread(self.backfill_images_batch, batch)
                if img:
                    logger.info("Captured images for %d scanned exhibits", img)
                # If we did a full batch there is likely more; loop quickly but
                # respect SEC rate limits. Otherwise idle longer.
                await asyncio.sleep(interval if n < batch else max(1.0, batch / rps))
            except asyncio.CancelledError:
                break
            except Exception as exc:  # noqa: BLE001
                logger.exception("backfill loop error: %s", exc)
                await asyncio.sleep(10)
        logger.info("Backfill worker stopped")


async def _d1_push_loop(db: Database, *, url: str, key: str, interval: float,
                        require_images: bool = False, batch: int = 100):
    """Periodically push *finalized* SQLite rows to D1 via the ingest route.

    SQLite stays the working buffer; D1 is authoritative. Failures here never
    disturb the listener/backfill — rows stay unmirrored and retry next cycle.

    Image capture is **decoupled** from the push: a row finalizes (and pushes) on
    markdown + metadata alone, so a slow/stalled image backfill never freezes the
    live feed. When images are captured later, ``update_image_urls`` re-queues the
    row so a follow-up push propagates them to D1 via the ingest upsert.
    """
    from .d1_sync import push_finalized

    logger.info("D1 push enabled -> %s (every %.0fs)", url, interval)
    while True:
        delay = interval
        try:
            n = await asyncio.to_thread(push_finalized, db, url, key,
                                        batch=batch, require_images=require_images)
            if n:
                logger.info("D1 push: sent %d finalized exhibits", n)
            if n >= batch:
                delay = 1.0  # full batch -> backlog likely; drain quickly, don't idle
        except Exception as exc:  # noqa: BLE001
            logger.warning("D1 push loop error: %s", exc)
        await asyncio.sleep(delay)


async def _run_all(config: Config):
    db = Database(config.db_path)
    db.init()
    listener = Listener(config, db)
    worker = BackfillWorker(db, image_token=os.environ.get("HF_TOKEN"))

    tasks = [
        asyncio.create_task(listener.run(), name="listener"),
        asyncio.create_task(worker.run(rps=config.requests_per_second), name="backfill"),
    ]

    # Push finalized rows to D1 (authoritative store) via the ingest route —
    # gated on the shared SEC_API_KEY. Without it, SQLite is the sole store.
    # Image capture is decoupled from the push (require_images=False default): rows
    # reach D1 on markdown+metadata, and re-push once images arrive (see
    # update_image_urls / _d1_push_loop). This keeps the live feed flowing even when
    # the image backfill is slow or stalled.
    if config.api_key:
        tasks.append(asyncio.create_task(
            _d1_push_loop(db, url=config.d1_ingest_url, key=config.api_key,
                          interval=float(os.environ.get("SEC_D1_PUSH_INTERVAL", "60"))),
            name="d1-push",
        ))

    # Optionally serve the internal API in-process, so one supervised process
    # covers listening, backfill, and the API. Still localhost/key-gated.
    api_server = None
    if config.serve_api:
        from .api import make_api_server

        api_server = make_api_server(config, db)
        tasks.append(asyncio.create_task(api_server.serve(), name="api"))
        logger.info("Serving internal API on %s:%d", config.api_host, config.api_port)

    stop = asyncio.Event()

    def _stop():
        listener.stop()
        stop.set()

    loop = asyncio.get_running_loop()
    for sig in (signal.SIGINT, signal.SIGTERM):
        try:
            loop.add_signal_handler(sig, _stop)
        except (NotImplementedError, RuntimeError):
            pass

    if config.run_duration_hours == 0:
        # Run until a signal arrives.
        await stop.wait()
    # When the listener finishes (duration reached) or a signal arrives, wind down.
    if api_server is not None:
        api_server.should_exit = True
    await asyncio.gather(tasks[0], return_exceptions=True)
    for task in tasks[1:]:
        task.cancel()
    await asyncio.gather(*tasks[1:], return_exceptions=True)


def main():
    logging.basicConfig(
        level=logging.INFO, format="%(asctime)s [%(levelname)s] %(name)s: %(message)s"
    )
    config = Config.from_env()
    asyncio.run(_run_all(config))


if __name__ == "__main__":
    main()
