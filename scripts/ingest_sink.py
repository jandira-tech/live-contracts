#!/usr/bin/env python3
"""Local ingest capture sink for comparing the Rust backend against the Python one.

Stands in for the production D1 `/api/ingest` route. Every POSTed row is appended
to a JSONL file (with a server-side receive timestamp) so we can diff what the Rust
backend produces against what the Python worker writes to ex10_listener.db.

Returns the same `{"accepted": [<id>, ...]}` shape (a list of bare ids) the real
route returns, so the Rust poster counts them as accepted.

Usage: ingest_sink.py <port> <out.jsonl>
"""
import json
import sys
import time
# Single-threaded on purpose: requests are serialized, so concurrent appends to
# the capture file can't interleave/corrupt JSONL (no lock needed).
from http.server import BaseHTTPRequestHandler, HTTPServer

OUT_PATH = "rust_ingest_capture.jsonl"


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):  # silence default stderr logging
        pass

    def _json(self, code, obj):
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/health":
            self._json(200, {"status": "ok", "sink": True})
        else:
            self._json(404, {"error": "not found"})

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length) if length else b""
        try:
            payload = json.loads(raw or b"{}")
        except Exception as exc:  # noqa: BLE001
            self._json(400, {"error": f"bad json: {exc}"})
            return
        # Mirror the real route's validation so the sink can't crash on a
        # malformed body (which would silently drop capture data).
        if not isinstance(payload, dict):
            self._json(400, {"error": "expected a JSON object"})
            return
        rows = payload.get("rows", [])
        if not isinstance(rows, list):
            self._json(400, {"error": "expected { rows: [...] }"})
            return
        recv = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
        accepted = []
        with open(OUT_PATH, "a") as fh:
            for row in rows:
                if not isinstance(row, dict):
                    continue  # skip non-object rows rather than crash
                row["_sink_received_at"] = recv
                fh.write(json.dumps(row) + "\n")
                accepted.append(row.get("id"))  # bare id — matches the real route
        self._json(200, {"accepted": accepted})


def main():
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8092
    global OUT_PATH
    OUT_PATH = sys.argv[2] if len(sys.argv) > 2 else OUT_PATH
    server = HTTPServer(("127.0.0.1", port), Handler)
    print(f"ingest sink listening on 127.0.0.1:{port} -> {OUT_PATH}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
