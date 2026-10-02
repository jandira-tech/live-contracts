"""export_d1_to_hf pages D1 by text id (UUIDv7 and legacy ids) and parses wrangler output robustly."""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from export_d1_to_hf import fetch_all, parse_wrangler_json  # noqa: E402


def test_pages_by_quoted_text_id_until_empty():
    ids = ["019e7330-a4d3-71dc-af92-b35fefef4e58", "019e7331-0000-7000-8000-000000000000", "999.0"]
    seen_sql = []

    def fake_d1(sql):
        seen_sql.append(sql)
        last = sql.split("id > '")[1].split("'")[0]
        rest = [i for i in sorted(ids) if i > last]
        return [{"id": i} for i in rest[:2]]

    rows = fetch_all(fake_d1, page=2)
    assert [r["id"] for r in rows] == sorted(ids)
    assert all("id > '" in s for s in seen_sql), seen_sql


def test_last_id_is_escaped():
    captured = []

    def fake_d1(sql):
        captured.append(sql)
        return [{"id": "a'b"}] if len(captured) == 1 else []

    fetch_all(fake_d1, page=1)
    assert "id > 'a''b'" in captured[1]


def test_parses_wrangler_json_after_a_banner_with_brackets():
    out = '[wrangler] banner\n⛅️ wrangler 4.x\n[\n  {\n    "results": [{"id": "x"}],\n    "success": true\n  }\n]\n'
    assert parse_wrangler_json(out) == [{"id": "x"}]
