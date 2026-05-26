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
                 metadata_fetcher=None, request_delay: float = 0.12, sleep_fn=time.sleep):
        self.db = db
        # fetcher(accession, cik, filename) -> raw document content (str/bytes) or ""
        self.fetcher = fetcher or self._datamule_fetcher
        # metadata_fetcher(accession, cik) -> compact filing header dict
        self.metadata_fetcher = metadata_fetcher or self._datamule_metadata_fetcher
        self.convert_fn = convert_fn
        # Per-request throttle for BOTH backfill loops (markdown + metadata): SEC
        # caps clients at 10 req/s and a full batch fires that many sec.gov fetches
        # back-to-back. Injectable so tests don't actually sleep.
        self.request_delay = request_delay
        self._sleep = sleep_fn

    def backfill_batch(self, limit: int = 25) -> int:
        """Convert up to ``limit`` pending exhibits. Returns the number converted."""
        rows = self.db.exhibits_missing_markdown(limit=limit)
        converted = 0
        for row in rows:
            if self.request_delay:
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
            if self.request_delay:
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

    # --- live fetch ---------------------------------------------------------
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
                # If we did a full batch there is likely more; loop quickly but
                # respect SEC rate limits. Otherwise idle longer.
                await asyncio.sleep(interval if n < batch else max(1.0, batch / rps))
            except asyncio.CancelledError:
                break
            except Exception as exc:  # noqa: BLE001
                logger.exception("backfill loop error: %s", exc)
                await asyncio.sleep(10)
        logger.info("Backfill worker stopped")


async def _hf_sync_loop(db: Database, *, token: str, repo: str, interval: float):
    """Periodically mirror ex10_exhibits to the HF dataset (parallel SQL sink).

    SQLite stays authoritative; failures here never disturb the listener/backfill.
    """
    from .hf_sync import sync_exhibits

    logger.info("HF dataset sync enabled -> %s (every %.0fs)", repo, interval)
    while True:
        try:
            n = await asyncio.to_thread(sync_exhibits, db, repo, token=token)
            if n:
                logger.info("HF sync: snapshotted %d exhibits to %s", n, repo)
        except Exception as exc:  # noqa: BLE001 - the mirror must never crash the worker
            logger.warning("HF dataset sync failed: %s", exc)
        # CancelledError (BaseException) propagates out to end the task — idiomatic.
        await asyncio.sleep(interval)


async def _run_all(config: Config):
    db = Database(config.db_path)
    db.init()
    listener = Listener(config, db)
    worker = BackfillWorker(db)

    tasks = [
        asyncio.create_task(listener.run(), name="listener"),
        asyncio.create_task(worker.run(rps=config.requests_per_second), name="backfill"),
    ]

    # Parallel HF dataset sink — opt-in via HF_TOKEN. Without it, SQLite is the
    # sole (plan-B) store and nothing here runs.
    hf_token = os.environ.get("HF_TOKEN")
    if hf_token:
        from .hf_sync import DATASET_REPO

        tasks.append(asyncio.create_task(
            _hf_sync_loop(
                db,
                token=hf_token,
                repo=os.environ.get("HF_DATASET_REPO", DATASET_REPO),
                interval=float(os.environ.get("SEC_HF_SYNC_INTERVAL", "900")),
            ),
            name="hf-sync",
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
