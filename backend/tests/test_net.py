"""Tests for retry/backoff robustness helper."""
import pytest

from sec_listener.net import retry_async


async def test_succeeds_after_transient_failures():
    attempts = {"n": 0}

    async def flaky():
        attempts["n"] += 1
        if attempts["n"] < 3:
            raise ConnectionError("boom")
        return "ok"

    result = await retry_async(flaky, retries=5, base_delay=0, sleep=_no_sleep)
    assert result == "ok"
    assert attempts["n"] == 3


async def test_raises_after_exhausting_retries():
    async def always_fails():
        raise TimeoutError("nope")

    with pytest.raises(TimeoutError):
        await retry_async(always_fails, retries=3, base_delay=0, sleep=_no_sleep)


async def test_uses_exponential_backoff_delays():
    delays = []

    async def record_sleep(d):
        delays.append(d)

    calls = {"n": 0}

    async def fails_twice():
        calls["n"] += 1
        if calls["n"] < 3:
            raise ConnectionError()
        return 42

    out = await retry_async(fails_twice, retries=5, base_delay=1.0, sleep=record_sleep)
    assert out == 42
    # Two retries -> two backoff sleeps, exponential: 1, 2
    assert delays == [1.0, 2.0]


async def _no_sleep(_d):
    return None
