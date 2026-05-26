"""One-shot boot step: restore SQLite from the HF dataset (durable source of truth).

The HF Space has no persistent disk, so a fresh container otherwise reseeds from
the frozen ``seed.db`` and loses everything captured since. With ``HF_TOKEN`` set,
this pulls the latest ``exhibits.parquet`` from the dataset and loads it into
SQLite when it has more rows than the seed — so restarts no longer lose data.

Run as: ``python -m sec_listener.boot_restore`` (in the entrypoint, before the
worker). No-op without ``HF_TOKEN``; never fails the boot.
"""
from __future__ import annotations

import logging
import os

from .config import Config
from .db import Database
from .hf_sync import DATASET_REPO, restore_from_dataset

logger = logging.getLogger(__name__)


def main() -> None:
    logging.basicConfig(level=logging.INFO, format="%(asctime)s [%(levelname)s] %(name)s: %(message)s")
    token = os.environ.get("HF_TOKEN")
    if not token:
        logger.info("boot restore skipped (no HF_TOKEN)")
        return
    config = Config.from_env()
    db = Database(config.db_path)
    db.init()
    repo = os.environ.get("HF_DATASET_REPO", DATASET_REPO)
    try:
        restore_from_dataset(db, repo, token=token)
    except Exception as exc:  # noqa: BLE001 - boot must proceed regardless
        logger.warning("boot restore failed (continuing): %s", exc)


if __name__ == "__main__":
    main()
