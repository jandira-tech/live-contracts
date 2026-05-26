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
import signal

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
    def __init__(self, db: Database, *, fetcher=None, convert_fn=convert_html_to_markdown):
        self.db = db
        # fetcher(accession, cik, filename) -> raw document content (str/bytes) or ""
        self.fetcher = fetcher or self._datamule_fetcher
        self.convert_fn = convert_fn

    def backfill_batch(self, limit: int = 25) -> int:
        """Convert up to ``limit`` pending exhibits. Returns the number converted."""
        rows = self.db.exhibits_missing_markdown(limit=limit)
        converted = 0
        for row in rows:
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

    # --- live fetch ---------------------------------------------------------
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
                # If we did a full batch there is likely more; loop quickly but
                # respect SEC rate limits. Otherwise idle longer.
                await asyncio.sleep(interval if n < batch else max(1.0, batch / rps))
            except asyncio.CancelledError:
                break
            except Exception as exc:  # noqa: BLE001
                logger.exception("backfill loop error: %s", exc)
                await asyncio.sleep(10)
        logger.info("Backfill worker stopped")


async def _run_all(config: Config):
    db = Database(config.db_path)
    db.init()
    listener = Listener(config, db)
    worker = BackfillWorker(db)

    tasks = [
        asyncio.create_task(listener.run(), name="listener"),
        asyncio.create_task(worker.run(rps=config.requests_per_second), name="backfill"),
    ]

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

    await stop.wait() if config.run_duration_hours == 0 else asyncio.sleep(0)
    # When the listener finishes (duration) or a signal arrives, wind down.
    await asyncio.gather(tasks[0], return_exceptions=True)
    tasks[1].cancel()
    await asyncio.gather(tasks[1], return_exceptions=True)


def main():
    logging.basicConfig(
        level=logging.INFO, format="%(asctime)s [%(levelname)s] %(name)s: %(message)s"
    )
    config = Config.from_env()
    asyncio.run(_run_all(config))


if __name__ == "__main__":
    main()
