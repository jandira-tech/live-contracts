"""Environment-driven configuration for the SEC listener."""
from __future__ import annotations

import os
from dataclasses import dataclass

_DEFAULT_UA = "SEC EX-10 Listener sec-monitor@example.com"


def _bool(val: str | None, default: bool) -> bool:
    if val is None:
        return default
    return val.strip().lower() in {"1", "true", "yes", "on"}


@dataclass(frozen=True)
class Config:
    db_path: str = "ex10_listener.db"
    poll_interval: int = 60
    run_duration_hours: float = 24
    requests_per_second: float = 5
    convert_markdown: bool = True
    user_agent: str = _DEFAULT_UA
    alert_file: str = "ex10_alerts.log"
    # Internal API (served in-process by the worker when serve_api is True).
    serve_api: bool = False
    api_host: str = "127.0.0.1"
    api_port: int = 8799
    api_key: str | None = None

    @property
    def min_request_interval(self) -> float:
        return 1.0 / self.requests_per_second if self.requests_per_second > 0 else 0.0

    @classmethod
    def from_env(cls) -> "Config":
        return cls(
            db_path=os.environ.get("SEC_DB_PATH", "ex10_listener.db"),
            poll_interval=int(os.environ.get("SEC_POLL_INTERVAL", "60")),
            run_duration_hours=float(os.environ.get("SEC_RUN_HOURS", "24")),
            requests_per_second=float(os.environ.get("SEC_RPS", "5")),
            convert_markdown=_bool(os.environ.get("SEC_CONVERT_MARKDOWN"), True),
            user_agent=os.environ.get("SEC_USER_AGENT", _DEFAULT_UA),
            alert_file=os.environ.get("SEC_ALERT_FILE", "ex10_alerts.log"),
            serve_api=_bool(os.environ.get("SEC_SERVE_API"), False),
            api_host=os.environ.get("SEC_API_HOST", "127.0.0.1"),
            api_port=int(os.environ.get("SEC_API_PORT", "8799")),
            api_key=os.environ.get("SEC_API_KEY") or None,
        )
