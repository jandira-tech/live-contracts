"""check_freshness: fail when /stats.json's latest_detected_at is more than two business days old."""
import datetime as dt
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from check_freshness import business_days_between, verdict  # noqa: E402

UTC = dt.timezone.utc


def at(s):
    return dt.datetime.fromisoformat(s).replace(tzinfo=UTC)


def test_business_days_skip_weekends():
    # Friday evening to Monday morning is one business day, not three calendar days.
    assert business_days_between(at("2026-10-02T20:00:00"), at("2026-10-05T09:00:00")) == 1
    assert business_days_between(at("2026-10-05T09:00:00"), at("2026-10-05T18:00:00")) == 0
    assert business_days_between(at("2026-09-28T10:00:00"), at("2026-10-02T10:00:00")) == 4


def test_fresh_and_stale():
    now = at("2026-10-07T12:00:00")  # Wednesday
    ok, _ = verdict({"latest_detected_at": "2026-10-05T15:13:04.47+00:00"}, now)
    assert ok
    ok, msg = verdict({"latest_detected_at": "2026-08-07T15:13:04.472868263+00:00"}, now)
    assert not ok and "2026-08-07" in msg


def test_unreadable_stats_fail_closed():
    now = at("2026-10-07T12:00:00")
    for bad in ({}, {"latest_detected_at": None}, {"latest_detected_at": "not a date"}):
        ok, _ = verdict(bad, now)
        assert not ok, bad
