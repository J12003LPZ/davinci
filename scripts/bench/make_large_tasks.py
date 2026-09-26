"""Generate the frozen multi-file benchmark fixtures.

The legacy generator is intentionally left untouched.  This module owns only
the larger ``m*`` stratum and the small offline specialist-reachability
fixtures used by benchmark contract tests.
"""
from __future__ import annotations

import hashlib
import json
import shutil
import textwrap
from pathlib import Path


ROOT = Path(__file__).resolve().parent
TASK_ROOT = ROOT / "tasks"
SPECIALIST_ROOT = ROOT / "specialist_fixtures"
LEGACY = {
    "t1-intervals", "t2-duration", "t3-lru", "t4-rename",
    "t5-csv", "t6-bookings", "t7-cli", "t8-calc",
}


def write_text(base: Path, relative: str, value: str) -> None:
    path = base / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(textwrap.dedent(value).lstrip("\n"), encoding="utf-8", newline="\n")


def tree_hash(base: Path) -> str:
    digest = hashlib.sha256()
    for path in sorted(p for p in base.rglob("*") if p.is_file()):
        relative = path.relative_to(base).as_posix()
        digest.update(relative.encode("utf-8"))
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_task(spec: dict[str, object]) -> dict[str, object]:
    task_id = str(spec["id"])
    if task_id in LEGACY or not task_id.startswith("m"):
        raise ValueError(f"large fixture id collides with legacy set: {task_id}")
    base = TASK_ROOT / task_id
    if base.exists():
        shutil.rmtree(base)
    repo = base / "repo"
    hidden = base / "hidden"
    solution = base / "solution"
    for relative, value in dict(spec["repo"]).items():
        write_text(repo, relative, value)
    for relative, value in dict(spec["hidden"]).items():
        write_text(hidden, relative, value)
    for relative, value in dict(spec["solution"]).items():
        write_text(solution, relative, value)
    metadata = {
        "id": task_id,
        "task_set": "large",
        "prompt": spec["prompt"],
        "allowed": spec["allowed"],
        "public_verification": spec["public_verification"],
    }
    write_json(base / "task.json", metadata)
    metadata.update({
        "public_hash": tree_hash(repo),
        "reference_solution_hash": tree_hash(solution),
        "hidden_grader_hash": tree_hash(hidden),
    })
    write_json(base / "task.json", metadata)
    return metadata


TASK_SPECS: list[dict[str, object]] = [
    {
        "id": "m1-config-parser",
        "prompt": (
            "Implement the configuration loader used by the deployment CLI. "
            "Parse UTF-8 key=value files with comments and blank lines, trim "
            "keys and values, let the last duplicate key win, and reject malformed "
            "or empty keys with ValueError. load_config(defaults, text, overrides) "
            "must return a new mapping with file values over defaults and explicit "
            "overrides over the file. Keep read_config as the compatibility alias, "
            "do not mutate caller mappings, and make the CLI output deterministic."
        ),
        "allowed": [
            "config_parser.py", "config_loader.py", "config_cli.py",
            "tests/test_public_config.py", "README.md",
        ],
        "public_verification": "python -m pytest -q tests/test_public_config.py",
        "repo": {
            "README.md": """
                # Configuration parser

                Configuration files contain one `key=value` pair per line.
            """,
            "config_parser.py": """
                def parse_config(text):
                    result = {}
                    for line in text.splitlines():
                        line = line.strip()
                        if not line or line.startswith("#"):
                            continue
                        key, value = line.split("=", 1)
                        result[key] = value
                    return result


                def read_config(text):
                    return parse_config(text)
            """,
            "config_loader.py": """
                from config_parser import parse_config


                def load_config(defaults, text, overrides=None):
                    result = defaults
                    result.update(overrides or {})
                    result.update(parse_config(text))
                    return result
            """,
            "config_cli.py": """
                import sys

                from config_parser import parse_config


                def main(argv=None):
                    path = (argv or sys.argv[1:])[0]
                    values = parse_config(open(path, encoding="utf-8").read())
                    for key, value in values.items():
                        print(f"{key}={value}")
                    return 0


                if __name__ == "__main__":
                    raise SystemExit(main())
            """,
            "tests/test_public_config.py": """
                from config_loader import load_config
                from config_parser import parse_config, read_config


                def test_basic_pairs_and_compatibility_alias():
                    text = "host=localhost\\nport=8080\\n"
                    assert parse_config(text) == {"host": "localhost", "port": "8080"}
                    assert read_config(text) == parse_config(text)


                def test_file_values_override_defaults():
                    assert load_config({"port": "80"}, "port=443\\n") == {"port": "443"}
            """,
        },
        "hidden": {
            "test_hidden_config.py": """
                import copy
                import pytest
                from config_loader import load_config
                from config_parser import parse_config


                def test_whitespace_comments_bom_and_duplicate_last_wins():
                    text = "\\ufeff # comment\\n host = local \\n host=prod\\n\\n"
                    assert parse_config(text) == {"host": "prod"}


                @pytest.mark.parametrize("text", ["broken", "=value", "  = value"])
                def test_invalid_syntax(text):
                    with pytest.raises(ValueError):
                        parse_config(text)


                def test_empty_input_and_precedence_do_not_mutate_callers():
                    defaults = {"host": "localhost", "port": "80"}
                    before = copy.deepcopy(defaults)
                    result = load_config(defaults, "port=443\\n", {"host": "prod"})
                    assert result == {"host": "prod", "port": "443"}
                    assert defaults == before
                    assert load_config(defaults, "", {}) == defaults


                def test_values_keep_equals_after_first_separator():
                    assert parse_config("token=a=b=c\\n") == {"token": "a=b=c"}
            """,
        },
        "solution": {
            "config_parser.py": """
                def parse_config(text):
                    if not isinstance(text, str):
                        raise ValueError("configuration must be text")
                    result = {}
                    for raw in text.lstrip("\\ufeff").splitlines():
                        line = raw.strip()
                        if not line or line.startswith(("#", ";")):
                            continue
                        if "=" not in line:
                            raise ValueError("expected key=value")
                        key, value = line.split("=", 1)
                        key, value = key.strip(), value.strip()
                        if not key:
                            raise ValueError("empty configuration key")
                        result[key] = value
                    return result


                def read_config(text):
                    return parse_config(text)
            """,
            "config_loader.py": """
                from config_parser import parse_config


                def load_config(defaults, text, overrides=None):
                    result = dict(defaults or {})
                    result.update(parse_config(text or ""))
                    result.update(dict(overrides or {}))
                    return result
            """,
            "config_cli.py": """
                import argparse
                import sys

                from config_loader import load_config


                def main(argv=None):
                    parser = argparse.ArgumentParser(description="print normalized configuration")
                    parser.add_argument("path")
                    parser.add_argument("--set", dest="overrides", action="append", default=[])
                    args = parser.parse_args(argv)
                    overrides = {}
                    for item in args.overrides:
                        if "=" not in item:
                            parser.error("--set requires key=value")
                        key, value = item.split("=", 1)
                        if not key.strip():
                            parser.error("--set requires a nonempty key")
                        overrides[key.strip()] = value.strip()
                    try:
                        text = open(args.path, encoding="utf-8-sig").read()
                        values = load_config({}, text, overrides)
                    except (OSError, ValueError) as error:
                        parser.error(str(error))
                    for key in sorted(values):
                        print(f"{key}={values[key]}")
                    return 0


                if __name__ == "__main__":
                    sys.exit(main())
            """,
        },
    },
    {
        "id": "m2-public-api-migration",
        "prompt": (
            "Migrate userlib's public formatter from format_user(name, active=True) "
            "to render_user(name, *, active=True, uppercase=False). Update every "
            "internal caller, export, CLI path, documentation example, and integration "
            "test. Preserve the promised compatibility shim at userlib.core.format_user "
            "for old callers, but do not keep stale internal imports or re-export the old "
            "name from userlib. Preserve validation and error propagation."
        ),
        "allowed": [
            "userlib/core.py", "userlib/exports.py", "userlib/reports.py",
            "userlib/notifications.py", "userlib/cli_helpers.py", "userlib/api.py",
            "userlib/__init__.py", "cli.py", "README.md", "tests/test_public_api.py",
        ],
        "public_verification": "python -m pytest -q tests/test_public_api.py",
        "repo": {
            "README.md": """
                # User library

                `format_user(name, active=True)` renders a display label.
            """,
            "userlib/__init__.py": """
                from .core import format_user

                __all__ = ["format_user"]
            """,
            "userlib/core.py": """
                def format_user(name, active=True):
                    if not isinstance(name, str) or not name:
                        raise ValueError("name is required")
                    return f"{name} ({'active' if active else 'inactive'})"
            """,
            "userlib/exports.py": """
                from .core import format_user


                def export_user(name):
                    return format_user(name)
            """,
            "userlib/reports.py": """
                from .core import format_user


                def report_user(name, active=True):
                    return format_user(name, active)
            """,
            "userlib/notifications.py": """
                from .core import format_user


                def notification(name):
                    return "User: " + format_user(name)
            """,
            "userlib/cli_helpers.py": """
                from .core import format_user


                def cli_label(name):
                    return format_user(name)
            """,
            "userlib/api.py": """
                from .core import format_user


                def api_payload(name):
                    return {"label": format_user(name)}
            """,
            "cli.py": """
                import sys

                from userlib.core import format_user


                def main(argv=None):
                    values = argv if argv is not None else sys.argv[1:]
                    print(format_user(values[0] if values else "CLI"))
                    return 0


                if __name__ == "__main__":
                    raise SystemExit(main())
            """,
            "tests/test_public_api.py": """
                from userlib import format_user
                from userlib.reports import report_user


                def test_legacy_public_behavior():
                    assert format_user("Ada") == "Ada (active)"
                    assert report_user("Ada", False) == "Ada (inactive)"
            """,
        },
        "hidden": {
            "test_hidden_api.py": """
                import pathlib
                import pytest
                import userlib
                from userlib import api, cli_helpers, exports, notifications, reports
                from userlib.core import format_user, render_user


                def test_new_signature_and_compatibility_shim():
                    assert render_user(" Ada ", uppercase=True) == "ADA (active)"
                    assert render_user("Ada", active=False) == "Ada (inactive)"
                    assert format_user("Ada") == render_user("Ada")
                    assert not hasattr(userlib, "format_user")


                @pytest.mark.parametrize("module, function", [
                    (exports, exports.export_user),
                    (reports, reports.report_user),
                    (notifications, notifications.notification),
                    (cli_helpers, cli_helpers.cli_label),
                    (api, api.api_payload),
                ])
                def test_all_callers_use_new_behavior(module, function):
                    assert "Ada (active)" in str(function("Ada"))


                def test_validation_errors_propagate():
                    with pytest.raises(ValueError):
                        render_user("   ")


                def test_no_stale_internal_imports_or_exports():
                    root = pathlib.Path(__file__).parent
                    for path in root.rglob("*.py"):
                        if "hidden" in path.name or path == pathlib.Path(__file__):
                            continue
                        text = path.read_text(encoding="utf-8")
                        if path.name == "core.py":
                            assert "def format_user" in text
                            continue
                        assert "from .core import format_user" not in text
                        assert "from userlib.core import format_user" not in text
                    assert userlib.__all__ == ["render_user"]
            """,
        },
        "solution": {
            "README.md": """
                # User library

                `render_user(name, *, active=True, uppercase=False)` is the public formatter.
                Old code can call `userlib.core.format_user` as a compatibility shim.
            """,
            "userlib/__init__.py": """
                from .core import render_user

                __all__ = ["render_user"]
            """,
            "userlib/core.py": """
                def render_user(name, *, active=True, uppercase=False):
                    if not isinstance(name, str) or not name.strip():
                        raise ValueError("name is required")
                    label = name.strip().upper() if uppercase else name.strip()
                    return f"{label} ({'active' if active else 'inactive'})"


                def format_user(name, active=True):
                    "Compatibility shim for callers that have not migrated yet."
                    return render_user(name, active=active)
            """,
            "userlib/exports.py": """
                from .core import render_user


                def export_user(name):
                    return render_user(name)
            """,
            "userlib/reports.py": """
                from .core import render_user


                def report_user(name, active=True):
                    return render_user(name, active=active)
            """,
            "userlib/notifications.py": """
                from .core import render_user


                def notification(name):
                    return "User: " + render_user(name)
            """,
            "userlib/cli_helpers.py": """
                from .core import render_user


                def cli_label(name):
                    return render_user(name)
            """,
            "userlib/api.py": """
                from .core import render_user


                def api_payload(name):
                    return {"label": render_user(name)}
            """,
            "cli.py": """
                import sys

                from userlib.core import render_user


                def main(argv=None):
                    values = argv if argv is not None else sys.argv[1:]
                    print(render_user(values[0] if values else "CLI"))
                    return 0


                if __name__ == "__main__":
                    raise SystemExit(main())
            """,
            "tests/test_public_api.py": """
                from userlib.core import render_user
                from userlib.reports import report_user


                def test_render_api():
                    assert render_user("Ada") == "Ada (active)"
                    assert report_user("Ada", False) == "Ada (inactive)"
            """,
        },
    },
    {
        "id": "m3-state-persistence",
        "prompt": (
            "Fix the counter service across state_core and state_persistence. "
            "apply(scope, request_id, delta) must validate input, persist atomically, "
            "be idempotent for a repeated request_id within the same scope, preserve "
            "event ordering, and leave the prior file intact if persistence fails. "
            "Separate scopes must never share request IDs or values; restart must load "
            "the same state. Do not mutate caller dictionaries."
        ),
        "allowed": [
            "state_core/model.py", "state_core/service.py",
            "state_persistence/json_store.py", "state_persistence/transaction.py",
            "app.py", "tests/test_public_state.py",
        ],
        "public_verification": "python -m pytest -q tests/test_public_state.py",
        "repo": {
            "state_core/__init__.py": "from .service import CounterService\n",
            "state_core/model.py": """
                def normalize_delta(delta):
                    return dict(delta)
            """,
            "state_core/service.py": """
                from state_core.model import normalize_delta


                class CounterService:
                    def __init__(self, store):
                        self.store = store

                    def apply(self, scope, request_id, delta):
                        state = self.store.load()
                        values = state.setdefault("scopes", {}).setdefault(scope, {})
                        for key, amount in normalize_delta(delta).items():
                            values[key] = values.get(key, 0) + amount
                        state.setdefault("requests", {})[request_id] = scope
                        state.setdefault("history", []).append({"scope": scope, "request_id": request_id})
                        self.store.save(state)
                        return values

                    def current(self, scope):
                        return self.store.load().get("scopes", {}).get(scope, {})
            """,
            "state_persistence/__init__.py": "from .json_store import JsonStore\n",
            "state_persistence/transaction.py": """
                def write_text(path, text):
                    path.write_text(text, encoding="utf-8")
            """,
            "state_persistence/json_store.py": """
                import json


                class JsonStore:
                    def __init__(self, path):
                        self.path = path

                    def load(self):
                        if not self.path.exists():
                            return {"scopes": {}, "requests": {}, "history": []}
                        return json.loads(self.path.read_text(encoding="utf-8"))

                    def save(self, state):
                        self.path.write_text(json.dumps(state), encoding="utf-8")
            """,
            "app.py": """
                import sys

                from state_core import CounterService
                from state_persistence import JsonStore


                def main(argv=None):
                    path, scope, request_id, key, amount = (argv or sys.argv[1:])
                    service = CounterService(JsonStore(__import__("pathlib").Path(path)))
                    print(service.apply(scope, request_id, {key: int(amount)}))
                    return 0


                if __name__ == "__main__":
                    raise SystemExit(main())
            """,
            "tests/test_public_state.py": """
                from pathlib import Path
                from state_core import CounterService
                from state_persistence import JsonStore


                def test_basic_apply_and_restart(tmp_path):
                    path = Path(tmp_path) / "state.json"
                    service = CounterService(JsonStore(path))
                    assert service.apply("team", "r1", {"done": 1}) == {"done": 1}
                    assert CounterService(JsonStore(path)).current("team") == {"done": 1}
            """,
        },
        "hidden": {
            "test_hidden_state.py": """
                from pathlib import Path
                import pytest
                from state_core import CounterService
                from state_persistence import JsonStore


                def service(tmp_path):
                    return CounterService(JsonStore(Path(tmp_path) / "state.json"))


                def test_idempotence_scope_and_ordering(tmp_path):
                    s = service(tmp_path)
                    assert s.apply("a", "same", {"n": 2}) == {"n": 2}
                    assert s.apply("a", "same", {"n": 99}) == {"n": 2}
                    assert s.apply("b", "same", {"n": 5}) == {"n": 5}
                    assert s.apply("a", "next", {"n": 3}) == {"n": 5}
                    state = s.store.load()
                    assert [e["request_id"] for e in state["history"]] == ["same", "same", "next"]
                    assert state["scopes"] == {"a": {"n": 5}, "b": {"n": 5}}


                def test_restart_and_caller_immutability(tmp_path):
                    s = service(tmp_path)
                    delta = {"x": 4}
                    s.apply("a", "r1", delta)
                    assert delta == {"x": 4}
                    assert service(tmp_path).current("a") == {"x": 4}


                def test_failed_replace_rolls_back(tmp_path, monkeypatch):
                    s = service(tmp_path)
                    s.apply("a", "r1", {"x": 1})
                    before = s.store.path.read_bytes()
                    original = __import__("state_persistence.json_store", fromlist=["os"]).os.replace

                    def fail(*args, **kwargs):
                        raise OSError("disk full")

                    monkeypatch.setattr("state_persistence.json_store.os.replace", fail)
                    with pytest.raises(OSError):
                        s.apply("a", "r2", {"x": 2})
                    assert s.store.path.read_bytes() == before
                    monkeypatch.setattr("state_persistence.json_store.os.replace", original)


                @pytest.mark.parametrize("scope,request_id,delta", [("", "r", {"x": 1}), ("a", "", {"x": 1}), ("a", "r", {})])
                def test_invalid_inputs(scope, request_id, delta, tmp_path):
                    with pytest.raises(ValueError):
                        service(tmp_path).apply(scope, request_id, delta)
            """,
        },
        "solution": {
            "state_core/__init__.py": "from .service import CounterService\n",
            "state_core/model.py": """
                def normalize_delta(delta):
                    if not isinstance(delta, dict) or not delta:
                        raise ValueError("delta must be a nonempty mapping")
                    result = {}
                    for key, amount in delta.items():
                        if not isinstance(key, str) or not key or type(amount) is not int:
                            raise ValueError("delta keys and values must be valid")
                        result[key] = amount
                    return result
            """,
            "state_core/service.py": """
                import copy

                from state_core.model import normalize_delta


                class CounterService:
                    def __init__(self, store):
                        self.store = store

                    def apply(self, scope, request_id, delta):
                        if not isinstance(scope, str) or not scope:
                            raise ValueError("scope is required")
                        if not isinstance(request_id, str) or not request_id:
                            raise ValueError("request_id is required")
                        normalized = normalize_delta(delta)
                        state = self.store.load()
                        scopes = state.setdefault("scopes", {})
                        requests = state.setdefault("requests", {})
                        history = state.setdefault("history", [])
                        scope_requests = requests.setdefault(scope, {})
                        if request_id in scope_requests:
                            return copy.deepcopy(scopes.get(scope, {}))
                        values = dict(scopes.get(scope, {}))
                        for key, amount in normalized.items():
                            values[key] = values.get(key, 0) + amount
                        candidate = copy.deepcopy(state)
                        candidate["scopes"][scope] = values
                        candidate["requests"].setdefault(scope, {})[request_id] = True
                        candidate["history"].append({
                            "scope": scope, "request_id": request_id,
                            "delta": dict(normalized),
                        })
                        self.store.save(candidate)
                        return copy.deepcopy(values)

                    def current(self, scope):
                        return copy.deepcopy(self.store.load().get("scopes", {}).get(scope, {}))
            """,
            "state_persistence/__init__.py": "from .json_store import JsonStore\n",
            "state_persistence/transaction.py": """
                import os
                import tempfile


                def write_text(path, text):
                    path.parent.mkdir(parents=True, exist_ok=True)
                    handle, temporary = tempfile.mkstemp(prefix=path.name + ".", dir=path.parent)
                    try:
                        with open(handle, "w", encoding="utf-8", closefd=True) as output:
                            output.write(text)
                            output.flush()
                            os.fsync(output.fileno())
                        os.replace(temporary, path)
                    except BaseException:
                        try:
                            os.unlink(temporary)
                        except FileNotFoundError:
                            pass
                        raise
            """,
            "state_persistence/json_store.py": """
                import json
                import os

                from state_persistence.transaction import write_text


                class JsonStore:
                    def __init__(self, path):
                        self.path = path

                    def load(self):
                        if not self.path.exists():
                            return {"version": 1, "scopes": {}, "requests": {}, "history": []}
                        state = json.loads(self.path.read_text(encoding="utf-8"))
                        if not isinstance(state, dict) or state.get("version") != 1:
                            raise ValueError("unsupported state file")
                        return state

                    def save(self, state):
                        payload = dict(state)
                        payload["version"] = 1
                        write_text(self.path, json.dumps(payload, sort_keys=True, separators=(",", ":")) + "\\n")
            """,
            "app.py": """
                import sys
                from pathlib import Path

                from state_core import CounterService
                from state_persistence import JsonStore


                def main(argv=None):
                    values = argv if argv is not None else sys.argv[1:]
                    if len(values) != 5:
                        raise SystemExit("usage: app.py STATE SCOPE REQUEST KEY AMOUNT")
                    path, scope, request_id, key, amount = values
                    service = CounterService(JsonStore(Path(path)))
                    print(service.apply(scope, request_id, {key: int(amount)}))
                    return 0


                if __name__ == "__main__":
                    raise SystemExit(main())
            """,
            "tests/test_public_state.py": """
                from pathlib import Path
                from state_core import CounterService
                from state_persistence import JsonStore


                def test_basic_apply_and_restart(tmp_path):
                    path = Path(tmp_path) / "state.json"
                    service = CounterService(JsonStore(path))
                    assert service.apply("team", "r1", {"done": 1}) == {"done": 1}
                    assert CounterService(JsonStore(path)).current("team") == {"done": 1}
            """,
        },
    },
    {
        "id": "m4-cli-feature",
        "prompt": (
            "Add the inventory report feature. Preserve the legacy `total PRICE QTY ...` "
            "command. Add `report NAME=PRICE:QTY ...` with --format text|json, --sort "
            "name|total, and --tax PERCENT (defaulting to INVENTORY_TAX or zero). "
            "Reject missing or malformed records, negative prices/quantities, and invalid "
            "tax values with exit status 2. JSON and text output must be deterministic; "
            "total sorting is descending with name as the tie breaker."
        ),
        "allowed": [
            "inventory/config.py", "inventory/args.py", "inventory/domain.py",
            "inventory/output.py", "cli.py", "tests/test_public_cli.py",
        ],
        "public_verification": "python -m pytest -q tests/test_public_cli.py",
        "repo": {
            "inventory/__init__.py": "",
            "inventory/config.py": """
                import os


                def load_config():
                    return {"tax": float(os.environ.get("INVENTORY_TAX", "0"))}
            """,
            "inventory/args.py": """
                import argparse


                def parser():
                    result = argparse.ArgumentParser()
                    result.add_argument("command", choices=["total", "report"])
                    result.add_argument("values", nargs="+")
                    return result
            """,
            "inventory/domain.py": """
                def parse_record(value):
                    name, numbers = value.split("=")
                    price, quantity = numbers.split(":")
                    return name, float(price), int(quantity)


                def report(records, tax=0):
                    rows = []
                    for record in records:
                        name, price, quantity = parse_record(record)
                        rows.append({"name": name, "quantity": quantity,
                                     "total": price * quantity * (1 + tax / 100)})
                    return rows
            """,
            "inventory/output.py": """
                import json


                def render(rows, fmt="text"):
                    if fmt == "json":
                        return json.dumps(rows)
                    return "\\n".join(f"{r['name']} {r['quantity']} {r['total']:.2f}" for r in rows)
            """,
            "cli.py": """
                import sys
                from inventory.args import parser
                from inventory.domain import report
                from inventory.output import render


                def main(argv=None):
                    args = parser().parse_args(argv)
                    if args.command == "total":
                        values = [float(v) for v in args.values]
                        print(f"{sum(values):.2f}")
                    else:
                        print(render(report(args.values)))
                    return 0


                if __name__ == "__main__":
                    raise SystemExit(main())
            """,
            "tests/test_public_cli.py": """
                import cli


                def test_legacy_total(capsys):
                    assert cli.main(["total", "10", "2"]) == 0
                    assert capsys.readouterr().out == "12.00\\n"
            """,
        },
        "hidden": {
            "test_hidden_cli.py": """
                import json
                import pytest
                import cli


                def test_report_json_and_name_order(capsys):
                    assert cli.main(["report", "Banana=2:1", "Apple=3:2", "--format", "json", "--sort", "name"]) == 0
                    rows = json.loads(capsys.readouterr().out)
                    assert [row["name"] for row in rows] == ["Apple", "Banana"]
                    assert rows[0]["total"] == 6.0


                def test_total_sort_and_tax(monkeypatch, capsys):
                    monkeypatch.setenv("INVENTORY_TAX", "10")
                    cli.main(["report", "A=10:1", "B=4:3", "--sort", "total"])
                    assert capsys.readouterr().out.splitlines() == ["B 3 13.20", "A 1 11.00"]


                @pytest.mark.parametrize("argv", [
                    ["report"], ["report", "bad"], ["report", "A=-1:2"],
                    ["report", "A=1:-2"], ["report", "A=1:2", "--tax", "bad"],
                ])
                def test_invalid_inputs_exit_two(argv):
                    with pytest.raises(SystemExit) as error:
                        cli.main(argv)
                    assert error.value.code == 2


                def test_legacy_total_still_works(capsys):
                    cli.main(["total", "10", "2", "5", "3"])
                    assert capsys.readouterr().out == "35.00\\n"
            """,
        },
        "solution": {
            "inventory/__init__.py": "",
            "inventory/config.py": """
                import os


                def load_config():
                    raw = os.environ.get("INVENTORY_TAX", "0")
                    try:
                        tax = float(raw)
                    except ValueError as error:
                        raise ValueError("invalid INVENTORY_TAX") from error
                    if tax < 0:
                        raise ValueError("tax must be nonnegative")
                    return {"tax": tax}
            """,
            "inventory/args.py": """
                import argparse


                def nonnegative(text):
                    try:
                        value = float(text)
                    except ValueError as error:
                        raise argparse.ArgumentTypeError("must be numeric") from error
                    if value < 0:
                        raise argparse.ArgumentTypeError("must be nonnegative")
                    return value


                def parser():
                    result = argparse.ArgumentParser()
                    result.add_argument("command", choices=["total", "report"])
                    result.add_argument("values", nargs="+")
                    result.add_argument("--format", choices=["text", "json"], default="text")
                    result.add_argument("--sort", choices=["name", "total"], default="name")
                    result.add_argument("--tax", type=nonnegative)
                    return result
            """,
            "inventory/domain.py": """
                def parse_record(value):
                    try:
                        name, numbers = value.split("=", 1)
                        price, quantity = numbers.split(":", 1)
                        price, quantity = float(price), int(quantity)
                    except (AttributeError, TypeError, ValueError) as error:
                        raise ValueError("record must be NAME=PRICE:QTY") from error
                    if not name or price < 0 or quantity < 0:
                        raise ValueError("record values must be nonnegative")
                    return name, price, quantity


                def report(records, tax=0, sort_by="name"):
                    if tax < 0:
                        raise ValueError("tax must be nonnegative")
                    rows = []
                    for record in records:
                        name, price, quantity = parse_record(record)
                        rows.append({"name": name, "quantity": quantity,
                                     "unit_price": price,
                                     "total": price * quantity * (1 + tax / 100)})
                    if sort_by == "total":
                        return sorted(rows, key=lambda row: (-row["total"], row["name"]))
                    return sorted(rows, key=lambda row: row["name"])
            """,
            "inventory/output.py": """
                import json


                def render(rows, fmt="text"):
                    if fmt == "json":
                        return json.dumps(rows, sort_keys=True, separators=(",", ":"))
                    return "\\n".join(
                        f"{row['name']} {row['quantity']} {row['total']:.2f}" for row in rows
                    )
            """,
            "cli.py": """
                import sys

                from inventory.args import parser
                from inventory.config import load_config
                from inventory.domain import report
                from inventory.output import render


                def main(argv=None):
                    args = parser().parse_args(argv)
                    try:
                        config = load_config()
                        tax = config["tax"] if args.tax is None else args.tax
                        if args.command == "total":
                            if len(args.values) == 0 or len(args.values) % 2:
                                raise ValueError("total requires PRICE QTY pairs")
                            numbers = [float(value) for value in args.values]
                            if any(value < 0 for value in numbers):
                                raise ValueError("total values must be nonnegative")
                            total = sum(numbers[index] * numbers[index + 1]
                                        for index in range(0, len(numbers), 2))
                            print(f"{total * (1 + tax / 100):.2f}")
                        else:
                            print(render(report(args.values, tax, args.sort), args.format))
                    except ValueError as error:
                        parser().error(str(error))
                    return 0


                if __name__ == "__main__":
                    raise SystemExit(main())
            """,
            "tests/test_public_cli.py": """
                import cli


                def test_legacy_total(capsys):
                    assert cli.main(["total", "10", "2"]) == 0
                    assert capsys.readouterr().out == "20.00\\n"
            """,
        },
    },
]


SPECIALIST_FIXTURES = {
    "discovery": {
        "kind": "tool-discovery",
        "offline": True,
        "preference_required": False,
        "registered": [
            {"name": "read", "family": "core", "authorized": True, "schema": True},
            {"name": "lsp_definition", "family": "lsp", "authorized": True, "schema": True},
            {"name": "browser_open", "family": "browser", "authorized": False, "schema": True},
        ],
        "queries": [
            {"mode": "family", "query": "lsp", "expected": ["lsp_definition"]},
            {"mode": "exact", "query": "browser_open", "expected": []},
        ],
    },
    "lsp": {
        "kind": "lsp-availability",
        "offline": True,
        "preference_required": False,
        "server": "python-lsp",
        "available": True,
        "workspace": "fixture-workspace",
        "expected": {"definition": "module.py:4", "references": 2},
    },
    "browser": {
        "kind": "browser-availability",
        "offline": True,
        "preference_required": False,
        "browsers": {
            "chrome": {"available": False, "reason": "fixture-only"},
            "edge": {"available": True, "version": "fixture"},
        },
        "expected": {"selected": "edge", "network": False},
    },
}


def write_specialist_fixtures() -> dict[str, dict[str, object]]:
    if SPECIALIST_ROOT.exists():
        shutil.rmtree(SPECIALIST_ROOT)
    entries = {}
    for name, fixture in SPECIALIST_FIXTURES.items():
        directory = SPECIALIST_ROOT / name
        write_json(directory / "fixture.json", fixture)
        entries[name] = {
            "path": f"specialist_fixtures/{name}/fixture.json",
            "kind": fixture["kind"],
            "offline": fixture["offline"],
            "preference_required": fixture["preference_required"],
            "fixture_hash": tree_hash(directory),
        }
    write_json(SPECIALIST_ROOT / "manifest.json", {
        "schema_version": 1,
        "fixtures": entries,
        "preference_is_functional_requirement": False,
    })
    return entries


def main() -> None:
    missing = [task for task in LEGACY if not (TASK_ROOT / task).is_dir()]
    if missing:
        raise SystemExit("legacy fixtures are missing: " + ", ".join(sorted(missing)))
    metadata = [write_task(spec) for spec in TASK_SPECS]
    specialists = write_specialist_fixtures()
    manifest = {
        "schema_version": 1,
        "task_set": "large",
        "generator": "scripts/bench/make_large_tasks.py",
        "tasks": [item["id"] for item in metadata],
        "fixtures": {
            item["id"]: {
                "task_set": item["task_set"],
                "allowed": item["allowed"],
                "public_verification": item["public_verification"],
                "public_hash": item["public_hash"],
                "reference_solution_hash": item["reference_solution_hash"],
                "hidden_grader_hash": item["hidden_grader_hash"],
            }
            for item in metadata
        },
        "specialist_fixtures": specialists,
    }
    write_json(ROOT / "large_manifest.json", manifest)
    print(json.dumps({"tasks": manifest["tasks"], "manifest": str(ROOT / "large_manifest.json")}, indent=2))


if __name__ == "__main__":
    main()
