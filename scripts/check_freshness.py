"""Fail when Live Contracts stops landing new exhibits.

    python scripts/check_freshness.py [https://live-contracts.arthur.law/stats.json]

Reads /stats.json and exits 1 when latest_detected_at is more than two business
days old, or when the stats cannot be read (fail closed). Run on a schedule by
.github/workflows/freshness.yml; a failed scheduled run emails the repo admins.
"""
import datetime as dt
import json
import re
import sys
import urllib.request

STATS_URL = "https://live-contracts.arthur.law/stats.json"
MAX_BUSINESS_DAYS = 2


def _parse(ts):
    # D1 stores nanoseconds; fromisoformat takes at most microseconds.
    ts = re.sub(r"(\.\d{6})\d+", r"\1", str(ts))
    t = dt.datetime.fromisoformat(ts.replace("Z", "+00:00"))
    return t if t.tzinfo else t.replace(tzinfo=dt.timezone.utc)


def business_days_between(a, b):
    """Weekdays strictly after a's date, up to and including b's date."""
    days, d = 0, a.date()
    while d < b.date():
        d += dt.timedelta(days=1)
        if d.weekday() < 5:
            days += 1
    return days


def verdict(stats, now):
    try:
        latest = _parse(stats["latest_detected_at"])
    except (KeyError, TypeError, ValueError):
        return False, f"stats.json has no readable latest_detected_at: {stats!r}"
    age = business_days_between(latest, now)
    msg = (f"latest_detected_at {latest.isoformat()} is {age} business day(s) old "
           f"(limit {MAX_BUSINESS_DAYS}); exhibits={stats.get('exhibits')}")
    return age <= MAX_BUSINESS_DAYS, msg


def main():
    url = sys.argv[1] if len(sys.argv) > 1 else STATS_URL
    try:
        req = urllib.request.Request(url, headers={"User-Agent": "live-contracts-freshness-check"})
        with urllib.request.urlopen(req, timeout=30) as r:
            stats = json.load(r)
    except Exception as e:  # noqa: BLE001 - any failure to read is a failure
        print(f"STALE: cannot read {url}: {e}")
        return 1
    ok, msg = verdict(stats, dt.datetime.now(dt.timezone.utc))
    print(("FRESH: " if ok else "STALE: ") + msg)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
