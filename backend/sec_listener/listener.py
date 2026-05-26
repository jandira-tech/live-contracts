"""Hardened, continuously-parsing SEC EX-10 RSS listener.

Responsibilities:
- Poll the SEC ``getcurrent`` Atom feed (paginated, rate-limited, retried).
- For each new filing, load the submission and classify documents.
- Persist traditional EX-10 exhibits (material contracts) with a Markdown
  rendering (via :mod:`sec_listener.converter`).
- Be robust: a single bad filing, network blip, or conversion error must never
  stop the loop.

The network-free orchestration core (:meth:`Listener.process_filing`) is unit
tested with an injected ``extractor``; the live datamule/aiohttp paths reuse the
battle-tested logic from the original script.
"""
from __future__ import annotations

import asyncio
import logging
import signal
import time
from datetime import datetime

import aiohttp

from .config import Config
from .converter import convert_html_to_markdown
from .db import Database
from .net import retry_async
from .parsing import classify_documents, extract_filing_header, parse_rss_feed

logger = logging.getLogger(__name__)

RSS_FEED_URL = "https://www.sec.gov/cgi-bin/browse-edgar?action=getcurrent&output=rss"


def _as_text(content) -> str:
    if content is None:
        return ""
    if isinstance(content, (bytes, bytearray)):
        return content.decode("utf-8", errors="replace")
    return str(content)


class Listener:
    def __init__(self, config: Config, db: Database, *, convert_fn=convert_html_to_markdown,
                 extractor=None):
        self.config = config
        self.db = db
        self.convert_fn = convert_fn
        # extractor(accession, cik) -> (ex10_docs, other_docs, filing_url)
        # ex10_docs items carry an extra "content" key (raw bytes/str).
        self.extractor = extractor or self._datamule_extractor
        self.running = False
        self.start_time = None
        self._last_request = 0.0

    # --- orchestration core (unit tested) ----------------------------------
    def process_filing(self, filing: dict) -> tuple[int, int]:
        accession = filing["accession"]
        cik = filing.get("cik", "")
        form_type = filing.get("form_type", "")

        if self.db.is_accession_seen(accession):
            return 0, 0

        # Mark seen up-front so a persistently broken filing is not retried forever.
        self.db.mark_accession_seen(accession, form_type, cik)
        self.db.save_rss_entry(
            accession, cik, form_type, filing.get("filing_date", ""), filing.get("summary", "")
        )

        try:
            ex10_docs, other_docs, filing_url, filing_metadata = self.extractor(accession, cik)
        except Exception as exc:  # noqa: BLE001 - robustness: never crash the loop
            logger.warning("extraction failed for %s: %s", accession, exc)
            return 0, 0

        for doc in ex10_docs:
            markdown = ""
            content = doc.get("content")
            if self.config.convert_markdown and content:
                try:
                    markdown = self.convert_fn(_as_text(content))
                except Exception as exc:  # noqa: BLE001
                    logger.warning("markdown conversion failed for %s: %s", accession, exc)
                    markdown = ""
            self.db.save_ex10_exhibit(
                {
                    "accession": accession,
                    "cik": cik,
                    "form_type": form_type,
                    "doc_type": doc.get("type", ""),
                    "filename": doc.get("filename", ""),
                    "description": doc.get("description", ""),
                    "sequence": doc.get("sequence", ""),
                    "url": filing_url,
                },
                markdown=markdown or None,
                filing_metadata=filing_metadata or None,
            )
            logger.info("EX-10 saved: %s %s (%s)", accession, doc.get("type"), doc.get("filename"))

        for doc in other_docs:
            self.db.save_all_exhibit(
                {
                    "accession": accession,
                    "cik": cik,
                    "form_type": form_type,
                    "doc_type": doc.get("type", ""),
                    "filename": doc.get("filename", ""),
                    "description": doc.get("description", ""),
                    "sequence": doc.get("sequence", ""),
                    "url": filing_url,
                }
            )

        return len(ex10_docs), len(other_docs)

    # --- live extraction (datamule) -----------------------------------------
    def _datamule_extractor(self, accession: str, cik: str):
        from datamule import Submission, format_accession

        formatted = format_accession(accession.replace("-", ""), "dash")
        url = f"https://www.sec.gov/Archives/edgar/data/{cik}/{formatted}.txt"
        sub = Submission(url=url)
        filing_metadata = extract_filing_header(sub.metadata.content)
        md_docs = sub.metadata.content.get("documents", [])
        ex10_meta, other_meta = classify_documents(md_docs)

        # Load raw content only for the EX-10 docs we will convert.
        ex10_types = [d.get("type") for d in ex10_meta]
        content_by_key: dict[tuple, object] = {}
        if ex10_types:
            try:
                for d in sub.document_type(ex10_types):
                    content_by_key[(d.type, d.filename)] = d.content
            except Exception as exc:  # noqa: BLE001
                logger.warning("loading EX-10 content failed for %s: %s", accession, exc)

        ex10_docs = []
        for d in ex10_meta:
            ex10_docs.append({**d, "content": content_by_key.get((d.get("type"), d.get("filename")))})

        return ex10_docs, other_meta, url, filing_metadata

    # --- live polling loop ---------------------------------------------------
    async def _rate_limit(self):
        elapsed = time.time() - self._last_request
        if elapsed < self.config.min_request_interval:
            await asyncio.sleep(self.config.min_request_interval - elapsed)
        self._last_request = time.time()

    async def _poll_rss(self, session: aiohttp.ClientSession) -> list[dict]:
        all_filings: list[dict] = []
        seen: set[str] = set()
        start = 0
        while True:
            await self._rate_limit()
            url = f"{RSS_FEED_URL}&count=100&start={start}"

            async def fetch():
                async with session.get(url, timeout=aiohttp.ClientTimeout(total=30)) as resp:
                    if resp.status != 200:
                        raise aiohttp.ClientResponseError(
                            resp.request_info, resp.history, status=resp.status
                        )
                    return await resp.text()

            try:
                content = await retry_async(fetch, retries=3, base_delay=1.0)
            except Exception as exc:  # noqa: BLE001
                logger.warning("RSS page fetch failed at start=%d: %s", start, exc)
                break

            filings = parse_rss_feed(content)
            if not filings:
                break
            for f in filings:
                acc = f.get("accession", "")
                if acc and acc not in seen:
                    seen.add(acc)
                    all_filings.append(f)
            if len(filings) < 100:
                break
            start += 100
            await asyncio.sleep(0.25)

        logger.info("RSS poll: %d filings", len(all_filings))
        return all_filings

    async def run(self):
        self.running = True
        self.start_time = time.time()
        self.db.init()
        logger.info("Listener starting (poll=%ds, run=%sh, markdown=%s)",
                    self.config.poll_interval, self.config.run_duration_hours,
                    self.config.convert_markdown)

        async with aiohttp.ClientSession(headers={"User-Agent": self.config.user_agent}) as session:
            while self.running:
                try:
                    if self.config.run_duration_hours:
                        elapsed_h = (time.time() - self.start_time) / 3600
                        if elapsed_h >= self.config.run_duration_hours:
                            logger.info("Run duration reached; stopping")
                            break

                    filings = await self._poll_rss(session)
                    new_ex10 = 0
                    for filing in filings:
                        # Blocking datamule work off the event loop.
                        ex10_c, _ = await asyncio.to_thread(self.process_filing, filing)
                        new_ex10 += ex10_c
                    if new_ex10:
                        logger.info("Saved %d new EX-10 exhibits this cycle", new_ex10)

                    await asyncio.sleep(self.config.poll_interval)
                except asyncio.CancelledError:
                    break
                except Exception as exc:  # noqa: BLE001 - loop must survive anything
                    logger.exception("monitoring loop error: %s", exc)
                    await asyncio.sleep(10)

        logger.info("Listener stopped")

    def stop(self, *_):
        self.running = False


def main():
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s [%(levelname)s] %(name)s: %(message)s",
    )
    config = Config.from_env()
    db = Database(config.db_path)
    db.init()
    listener = Listener(config, db)

    loop = asyncio.new_event_loop()
    asyncio.set_event_loop(loop)
    for sig in (signal.SIGINT, signal.SIGTERM):
        try:
            loop.add_signal_handler(sig, listener.stop)
        except (NotImplementedError, RuntimeError):
            pass
    try:
        loop.run_until_complete(listener.run())
    finally:
        loop.close()


if __name__ == "__main__":
    main()
