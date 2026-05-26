"""Network robustness helpers: retry with exponential backoff."""
from __future__ import annotations

import asyncio
import logging
from typing import Awaitable, Callable, TypeVar

logger = logging.getLogger(__name__)

T = TypeVar("T")


async def retry_async(
    factory: Callable[[], Awaitable[T]],
    *,
    retries: int = 3,
    base_delay: float = 0.5,
    sleep: Callable[[float], Awaitable[None]] = asyncio.sleep,
) -> T:
    """Call ``factory()`` (an async callable) until it succeeds.

    Retries on any exception with exponential backoff (base_delay * 2**attempt).
    Re-raises the last exception once ``retries`` is exhausted.
    """
    last_exc: Exception | None = None
    for attempt in range(retries):
        try:
            return await factory()
        except Exception as exc:  # noqa: BLE001 - transient network errors are expected
            last_exc = exc
            if attempt == retries - 1:
                break
            delay = base_delay * (2 ** attempt)
            logger.warning(
                "attempt %d/%d failed (%s); retrying in %.1fs", attempt + 1, retries, exc, delay
            )
            await sleep(delay)
    assert last_exc is not None
    raise last_exc
