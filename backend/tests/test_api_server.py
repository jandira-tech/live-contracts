"""Test the uvicorn server builder used to serve the API in-process."""
from sec_listener.api import make_api_server
from sec_listener.config import Config
from sec_listener.db import Database


def test_make_api_server_binds_configured_host_port(tmp_path):
    db = Database(str(tmp_path / "s.db"))
    db.init()
    cfg = Config(db_path=db.path, api_host="127.0.0.1", api_port=8123, api_key="k")
    server = make_api_server(cfg, db)
    # uvicorn.Server exposes its Config with the bind settings + ASGI app.
    assert server.config.host == "127.0.0.1"
    assert server.config.port == 8123
    assert server.config.app is not None
