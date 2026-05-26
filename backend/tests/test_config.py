"""Tests for env-driven configuration."""
from sec_listener.config import Config


def test_defaults_when_no_env(monkeypatch):
    for key in list(_ENV_KEYS):
        monkeypatch.delenv(key, raising=False)
    cfg = Config.from_env()
    assert cfg.db_path == "ex10_listener.db"
    assert cfg.poll_interval == 60
    assert cfg.run_duration_hours == 24
    assert cfg.requests_per_second == 5
    assert cfg.convert_markdown is True
    assert "sec" in cfg.user_agent.lower()


def test_reads_overrides_from_env(monkeypatch):
    monkeypatch.setenv("SEC_DB_PATH", "/tmp/custom.db")
    monkeypatch.setenv("SEC_POLL_INTERVAL", "15")
    monkeypatch.setenv("SEC_RUN_HOURS", "0")  # 0 => run forever
    monkeypatch.setenv("SEC_RPS", "8")
    monkeypatch.setenv("SEC_CONVERT_MARKDOWN", "false")
    cfg = Config.from_env()
    assert cfg.db_path == "/tmp/custom.db"
    assert cfg.poll_interval == 15
    assert cfg.run_duration_hours == 0
    assert cfg.requests_per_second == 8
    assert cfg.convert_markdown is False


def test_min_request_interval_derived_from_rps(monkeypatch):
    monkeypatch.setenv("SEC_RPS", "5")
    cfg = Config.from_env()
    assert abs(cfg.min_request_interval - 0.2) < 1e-9


_ENV_KEYS = (
    "SEC_DB_PATH",
    "SEC_POLL_INTERVAL",
    "SEC_RUN_HOURS",
    "SEC_RPS",
    "SEC_CONVERT_MARKDOWN",
    "SEC_USER_AGENT",
    "SEC_ALERT_FILE",
)
