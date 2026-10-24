"""Generate the frozen multi-file benchmark fixtures.

The legacy generator is intentionally left untouched.  This module owns only
the larger ``m*`` stratum and the small offline specialist-reachability
fixtures used by benchmark contract tests.
"""
from __future__ import annotations

import hashlib
import json
import math
import os
import pprint
import shutil
import subprocess
import sys
import tempfile
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
    for path in sorted((p for p in base.rglob("*") if p.is_file()),
                       key=lambda p: p.relative_to(base).as_posix()):
        relative = path.relative_to(base).as_posix()
        digest.update(relative.encode("utf-8"))
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8", newline="\n")


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


# --- m5-m7 builders begin ---


class Rng:
    """Small deterministic generator: fixture bytes must not depend on the Python version."""

    def __init__(self, seed: int) -> None:
        self.state = (seed * 2654435761 + 40503) & 0xFFFFFFFF

    def next(self) -> int:
        self.state = (self.state * 1664525 + 1013904223) & 0xFFFFFFFF
        return self.state >> 8

    def below(self, bound: int) -> int:
        return self.next() % bound

    def span(self, low: int, high: int) -> int:
        return low + self.below(high - low + 1)

    def pick(self, items):
        return items[self.below(len(items))]


def fill(template: str, **values: object) -> str:
    for key, value in values.items():
        template = template.replace("@" + key + "@", str(value))
    return template


def run_golden(files: dict[str, str], script: str) -> object:
    """Run `script` in a scratch tree holding `files`; return its JSON output."""
    with tempfile.TemporaryDirectory() as temporary:
        base = Path(temporary)
        for relative, value in files.items():
            path = base / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(value, encoding="utf-8", newline="\n")
        (base / "_golden.py").write_text(script, encoding="utf-8", newline="\n")
        done = subprocess.run(
            [sys.executable, "-W", "ignore", "_golden.py"], cwd=base, capture_output=True,
            text=True, encoding="utf-8", timeout=120,
            env=dict(os.environ, PYTHONHASHSEED="0", PYTHONUTF8="1", PYTHONDONTWRITEBYTECODE="1"))
        if done.returncode != 0:
            raise RuntimeError("golden run failed: " + done.stderr[-2000:])
        return json.loads(done.stdout)


def literal(value: object) -> str:
    return pprint.pformat(value, width=96, sort_dicts=True)


# ---------------------------------------------------------------------------
# m5-wide-migration
# ---------------------------------------------------------------------------

M5_DOMAINS = [
    "billing", "invoices", "payroll", "refunds", "shipping", "returns", "catalog", "pricing",
    "ledger", "payouts", "subscriptions", "coupons", "taxes", "disputes", "settlements",
    "quotes", "credits", "vouchers", "tipping", "escrow", "rewards", "fees", "rebates",
    "statements", "reserves", "audits", "budgets", "grants", "leases", "royalties",
]
M5_DECOYS = {6, 17, 25}
M5_CURRENCIES = ["EUR", "GBP", "CHF", "SEK", "CAD", "AUD", "NOK", "DKK", "PLN", "CZK"]
M5_FROZEN = "datetime(2026, 3, 14, 23, 30, tzinfo=timezone.utc)"
M5_PHRASES = [
    "Amounts are integer cents.", "Timestamps are stamped when the call is made.",
    "Rounding follows the table below.", "Callers pass plain values, never objects.",
    "The ledger keeps rows in insertion order.", "Negative amounts are refunds.",
    "Regions without a rate use the default.", "Totals are never cached.",
    "Validation errors are ValueError.", "Labels are shown to operators unchanged.",
    "The window helpers return text, not datetimes.", "Tiers apply to the whole quantity.",
]
M5_ROUNDING = {
    "half_up": "    if remainder * 2 >= 10000:\n        quotient += 1\n",
    "bankers": ("    if remainder * 2 > 10000 or (remainder * 2 == 10000 and quotient % 2):\n"
                "        quotient += 1\n"),
    "floor": "    # fractions of a cent are dropped\n",
    "ceil": "    if remainder:\n        quotient += 1\n",
}


class ServiceStyle:
    """How one service module reaches the clock and the money formatter."""

    def __init__(self, style: int, v2: bool, currency_name: str) -> None:
        self.style, self.v2, self.currency_name = style, v2, currency_name

    def imports(self) -> str:
        table = {
            (0, False): "from legacy_clock import now\nfrom legacy_money import fmt_money\n",
            (0, True): "from platform_api.clock import utc_now\nfrom platform_api.money import format_money\n",
            (1, False): "import legacy_clock\nimport legacy_money\n",
            (1, True): "from platform_api import clock, money\n",
            (2, False): "from legacy_clock import now as _now\nfrom legacy_money import fmt_money as _money\n",
            (2, True): ("from platform_api.clock import utc_now as _now\n"
                        "from platform_api.money import format_money as _money\n"),
            (3, False): "",
            (3, True): "",
            (4, False): "from legacy_clock import now\nfrom legacy_money import fmt_money\n",
            (4, True): "from platform_api.clock import utc_now\nfrom platform_api.money import format_money\n",
        }
        return table[(self.style, self.v2)]

    def lazy(self, clock: bool, money: bool, indent: int = 4) -> str:
        if self.style != 3:
            return ""
        lines = []
        if clock:
            lines.append("from platform_api.clock import utc_now" if self.v2 else "from legacy_clock import now")
        if money:
            lines.append("from platform_api.money import format_money" if self.v2
                         else "from legacy_money import fmt_money")
        return "".join(" " * indent + line + "\n" for line in lines)

    def now(self) -> str:
        names = {0: ("now", "utc_now"), 1: ("legacy_clock.now", "clock.utc_now"),
                 2: ("_now", "_now"), 3: ("now", "utc_now"), 4: ("now", "utc_now")}
        return names[self.style][self.v2] + "()"

    def money(self, argument: str) -> str:
        cur = self.currency_name
        if not self.v2:
            return {0: "fmt_money", 1: "legacy_money.fmt_money", 2: "_money", 3: "fmt_money",
                    4: "fmt_money"}[self.style] + f"({argument})"
        return {0: "format_money", 1: "money.format_money", 2: "_money", 3: "format_money",
                4: "format_money"}[self.style] + f"({argument}, {cur})"

    def money_map(self, source: str) -> str:
        """`list(map(...))` over `source`, the form that passes the function itself."""
        if not self.v2:
            return f"list(map(fmt_money, {source}))"
        return f"list(map(lambda cents: format_money(cents, {self.currency_name}), {source}))"


def m5_service(index: int, v2: bool) -> tuple[str, dict[str, object]]:
    """Source of services/<domain>.py for the starter (v2=False) or the reference (v2=True)."""
    rng = Rng(1000 + index)
    domain = M5_DOMAINS[index]
    title = domain.capitalize()
    decoy = index in M5_DECOYS
    style = ServiceStyle(index % 5, v2 and not decoy, "REPORTING_CURRENCY" if index % 7 == 3 else "CURRENCY")
    cur_name = style.currency_name
    currency = M5_CURRENCIES[rng.below(len(M5_CURRENCIES))]
    mode = ("half_up", "bankers", "floor", "ceil")[rng.below(4)]
    regions = [f"R{rng.span(1, 9)}{chr(65 + k)}" for k in range(rng.span(14, 22))]
    rates = ", ".join(f'"{r}": {rng.span(0, 2600)}' for r in regions)
    tiers = []
    quantity = 0
    for _ in range(rng.span(4, 6)):
        quantity += rng.span(5, 40)
        tiers.append((quantity, rng.span(25, 400) + len(tiers) * 150))
    tier_text = ", ".join(f"({q}, {bps})" for q, bps in tiers)
    required = rng.span(3, 5)
    pool = ["id", "owner", "amount", "currency", "channel", "region", "reference", "kind", "batch", "window"]
    keys = []
    while len(keys) < required:
        key = rng.pick(pool)
        if key not in keys:
            keys.append(key)
    limits = [(key, rng.span(1, 9) * 50) for key in keys]
    sentences = " ".join(rng.pick(M5_PHRASES) for _ in range(rng.span(3, 5)))
    rule_count = rng.span(10, 15)
    rules = []
    for k in range(rule_count):
        form = rng.below(4)
        a, b, m = rng.span(3, 97), rng.span(1, 250), rng.span(31, 997)
        if form == 0:
            rules.append(f"def _rule_{k}(value):\n    return (value * {a} + {b}) % {m}\n")
        elif form == 1:
            rules.append(f"def _rule_{k}(value):\n    if value % 2:\n        return (value ^ {a}) % {m}\n"
                         f"    return (value + {b}) % {m}\n")
        elif form == 2:
            rules.append(f"def _rule_{k}(value):\n    if value < {a}:\n        return {b}\n"
                         f"    if value < {a * 3}:\n        return {b * 2}\n    return {m} - {b}\n")
        else:
            rules.append(f"def _rule_{k}(value):\n    total = 0\n    for step in range({rng.span(2, 5)}):\n"
                         f"        total += (value >> step) & {a}\n    return total % {m}\n")
    rule_calls = ", ".join(f"_rule_{k}" for k in range(rule_count))
    label = f"{title} {rng.pick(['desk', 'queue', 'run', 'batch', 'office'])}"
    modulus = rng.span(97, 9973)
    pad = rng.span(5, 9)

    header = "# api: v2\n" if (v2 and not decoy) else ""
    out = [header + f'"""{title} service {index:02d}. {sentences}"""\n']
    out.append("from datetime import timedelta\n")
    out.append("" if decoy else style.imports())
    out.append(f'\n{cur_name} = "{currency}"  # reporting currency of this service\n')
    out.append(f"DEFAULT_BPS = {rng.span(0, 2000)}\n")
    out.append(f"RATE_TABLE = {{{rates}}}\n")
    out.append(f"TIERS = [{tier_text}]\n")
    out.append(f'REQUIRED = {tuple(keys)!r}\n')
    out.append(f"LIMITS = {dict(limits)!r}\n")
    names = [f"{domain}_stamp", f"{domain}_due", f"{domain}_window", f"{domain}_label",
             f"{domain}_lines", f"{domain}_tax", f"{domain}_tiers", f"{domain}_validate",
             f"{domain}_score", f"{domain}_digest", f"{domain}_demo", f"{title}Ledger"]
    out.append("__all__ = " + repr(names) + "\n\n\n")

    if decoy:
        out.append(
            "def now():\n"
            f'    """Tick label used by this desk; unrelated to legacy_clock.now()."""\n'
            f'    return "tick-{index:02d}"\n\n\n'
            "def fmt_money(cents):\n"
            '    """Local plain formatter; not the shared fmt_money(cents) helper."""\n'
            "    sign = '-' if cents < 0 else ''\n"
            "    return f\"{sign}{abs(cents) // 100}.{abs(cents) % 100:02d}\"\n\n\n")
        stamp_body = f'    return prefix + now() + "-" + "{index:02d}"\n'
        due_body = '    return f"day+{days + grace}"\n'
        window_body = '    return f"-{hours}h", now()\n'
        label_body = f'    return f"{label}: {{fmt_money(cents)}}"\n'
        lines_expr = "[fmt_money(a) for a in amounts]"
        demo_open, ledger_age = '"tick"', "0.0"
        ledger_open = "now()"
        total_expr = "fmt_money(sum(self.rows))"
        pre_c = pre_m = ""
    else:
        stamp_body = f'{style.lazy(True, False)}    return prefix + {style.now()}.strftime("%Y%m%dT%H%M")\n'
        due_body = (f"{style.lazy(True, False)}    due = {style.now()} + timedelta(days=days + grace)\n"
                    "    return due.date().isoformat()\n")
        window_body = (f"{style.lazy(True, False)}    end = {style.now()}\n"
                       "    start = end - timedelta(hours=hours)\n"
                       "    return start.isoformat(), end.isoformat()\n")
        label_body = f'{style.lazy(False, True)}    return f"{label}: {{{style.money("cents")}}}"\n'
        lines_expr = style.money_map("amounts") if style.style == 4 else f"[{style.money('a')} for a in amounts]"
        ledger_open = style.now()
        total_expr = style.money("sum(self.rows)")
        pre_c, pre_m = style.lazy(True, False, 8), style.lazy(False, True, 8)

    out.append(f'def {domain}_stamp(prefix=""):\n    """Reference stamp for new {domain} rows."""\n{stamp_body}\n\n')
    out.append(f"def {domain}_due(days, grace=0):\n    \"\"\"Due date, {domain} terms plus grace days.\"\"\"\n"
               f"    if days < 0 or grace < 0:\n        raise ValueError(\"negative terms\")\n{due_body}\n\n")
    out.append(f"def {domain}_window(hours):\n    \"\"\"Start and end of the reporting window.\"\"\"\n{window_body}\n\n")
    out.append(f"def {domain}_label(cents):\n    \"\"\"Operator label for one amount.\"\"\"\n{label_body}\n\n")
    out.append(
        f"def {domain}_lines(amounts):\n"
        f"    \"\"\"Numbered statement lines for {domain}.\"\"\"\n"
        f"{style.lazy(False, True) if not decoy else ''}"
        f"    texts = {lines_expr}\n"
        f"    return \"\\n\".join(f\"{{number:0{pad - 4 if pad > 5 else 2}d}} {{text}}\" "
        f"for number, text in enumerate(texts, 1))\n\n\n")
    out.append(
        f"def {domain}_tax(cents, region=None):\n"
        f"    \"\"\"Tax in cents; rounding is {mode.replace('_', ' ')}.\"\"\"\n"
        f"    if cents < 0:\n        raise ValueError(\"amount must be nonnegative\")\n"
        f"    bps = RATE_TABLE.get(region, DEFAULT_BPS)\n"
        f"    quotient, remainder = divmod(cents * bps, 10000)\n"
        f"{M5_ROUNDING[mode]}    return quotient\n\n\n")
    out.append(
        f"def {domain}_tiers(quantity, unit_cents):\n"
        f"    \"\"\"Total after the volume tier that fits `quantity`.\"\"\"\n"
        f"    if quantity <= 0 or unit_cents < 0:\n        raise ValueError(\"quantity must be positive\")\n"
        f"    discount_bps = 0\n"
        f"    for minimum, bps in TIERS:\n        if quantity >= minimum:\n            discount_bps = bps\n"
        f"    total = quantity * unit_cents\n"
        f"    return total - (total * discount_bps + {rng.pick([0, 5000])}) // 10000\n\n\n")
    checks = "".join(
        f"    if not isinstance(record.get(\"{key}\"), (int, str)) or record.get(\"{key}\") == \"\":\n"
        f"        raise ValueError(\"{key} is required\")\n"
        f"    if len(str(record[\"{key}\"])) > {limit}:\n        raise ValueError(\"{key} is too long\")\n"
        for key, limit in limits)
    out.append(f"def {domain}_validate(record):\n    \"\"\"Check one {domain} record.\"\"\"\n{checks}    return True\n\n\n")
    out.extend(rule + "\n\n" for rule in rules)
    out.append(
        f"def {domain}_score(value):\n"
        f"    \"\"\"Fold the module rules into one score.\"\"\"\n"
        f"    total = 0\n    for rule in ({rule_calls},):\n        total += rule(value)\n"
        f"    return total % {modulus}\n\n\n")
    out.append(
        f"def {domain}_digest(rows):\n"
        f"    \"\"\"Order-sensitive checksum of the row texts.\"\"\"\n"
        f"    acc = {rng.span(1, 90)}\n    for row in rows:\n        for char in str(row):\n"
        f"            acc = (acc * {rng.span(3, 41)} + ord(char)) % {modulus}\n    return acc\n\n\n")
    out.append(
        f"class {title}Ledger:\n"
        f"    \"\"\"Rows for one {domain} run.\"\"\"\n\n"
        f"    def __init__(self):\n{pre_c}        self.opened = {ledger_open}\n        self.rows = []\n\n"
        f"    def add(self, cents, note=\"\"):\n"
        f"        if not isinstance(cents, int):\n            raise ValueError(\"cents must be an integer\")\n"
        f"        self.rows.append(cents)\n        return len(self.rows)\n\n"
        f"    def total_label(self):\n{pre_m}        return {total_expr}\n\n"
        f"    def age_hours(self):\n{pre_c}        return {'0.0' if decoy else f'round(({style.now()} - self.opened).total_seconds() / 3600, 2)'}\n\n\n")
    out.append(
        f"def {domain}_demo():\n"
        f"    \"\"\"Small end-to-end run used by the smoke checks.\"\"\"\n"
        f"    ledger = {title}Ledger()\n    for cents in (1999, 250, 100000):\n        ledger.add(cents, \"demo\")\n"
        f"    return str(ledger.opened), ledger.total_label(), ledger.age_hours()\n")
    smoke = [f"{domain}_stamp('x-')", f"{domain}_due(3, 1)", f"{domain}_window(6)",
             f"{domain}_label(123456)", f"{domain}_label(-50)", f"{domain}_lines([5, 120000, -250])",
             f"{domain}_tax(10050, {regions[0]!r})", f"{domain}_tiers(37, 1999)", f"{domain}_demo()",
             f"{domain}_score(77)", f"{domain}_digest(['a', 'bc', 7])"]
    return "".join(out), {"name": domain, "decoy": decoy, "all": names, "smoke": smoke}


def m5_vendor(part: int) -> str:
    rng = Rng(5000 + part)
    sentences = " ".join(rng.pick(M5_PHRASES) for _ in range(3))
    mod = rng.span(50, 900)
    return fill('''
        """Vendored ledgerkit part @part@. @sentences@ Do not edit: owned upstream."""
        import legacy_clock
        from legacy_money import fmt_money

        STEP = @step@
        SHIFT = @shift@


        def stamp_@part@(prefix=""):
            return prefix + legacy_clock.now().strftime("%Y-%m-%d %H:%M:%S")


        def window_@part@(hours):
            end = legacy_clock.now()
            return (end.hour - hours) % 24, end.isoformat()


        def label_@part@(cents):
            return "@part@:" + fmt_money(cents * STEP)


        def spread_@part@(values):
            return [fmt_money(v + SHIFT) for v in values]


        def checksum_@part@(text):
            acc = @part@
            for char in text:
                acc = (acc * 31 + ord(char)) % @mod@
            return acc
    ''', part=part, sentences=sentences, step=rng.span(1, 9), shift=rng.span(10, 900), mod=mod)


M5_PLATFORM = {
    "platform_api/__init__.py": '"""Version 2 platform API: UTC clock and currency-aware money."""\n',
    "platform_api/clock.py": '''
        """Timezone-aware UTC clock. Tests freeze it with freeze()."""
        from datetime import datetime, timezone

        _frozen = None


        def freeze(value):
            global _frozen
            if value is not None and value.tzinfo is None:
                raise ValueError("frozen time must be timezone-aware")
            _frozen = value


        def utc_now():
            """Current time as an aware UTC datetime."""
            if _frozen is not None:
                return _frozen
            return datetime.now(timezone.utc)
    ''',
    "platform_api/money.py": '''
        """Currency-aware money formatting."""
        CURRENCIES = ("EUR", "GBP", "CHF", "SEK", "CAD", "AUD", "NOK", "DKK", "PLN", "CZK", "USD")


        def format_money(cents, currency):
            """1234567, "EUR" -> "12,345.67 EUR". Negative amounts keep a leading minus."""
            if currency not in CURRENCIES:
                raise ValueError(f"unknown currency: {currency!r}")
            sign = "-" if cents < 0 else ""
            whole, fraction = divmod(abs(int(cents)), 100)
            return f"{sign}{whole:,}.{fraction:02d} {currency}"
    ''',
    "legacy_money.py": '''
        """Old money formatter, still used by vendored code."""


        def fmt_money(cents):
            """1234567 -> "$12,345.67"."""
            sign = "-" if cents < 0 else ""
            whole, fraction = divmod(abs(int(cents)), 100)
            return f"{sign}${whole:,}.{fraction:02d}"
    ''',
}

M5_LEGACY_CLOCK = '''
    """Old clock. Superseded by platform_api.clock.utc_now()."""
    from datetime import timedelta

    from platform_api.clock import utc_now

    LEGACY_OFFSET = timedelta(hours=2)


    def now():
        """Naive local time of the original deployment (UTC+2)."""
        return (utc_now() + LEGACY_OFFSET).replace(tzinfo=None)
'''

M5_LEGACY_CLOCK_SHIM = '''
    """Compatibility shim. Superseded by platform_api.clock.utc_now()."""
    import warnings
    from datetime import timedelta

    from platform_api.clock import utc_now

    LEGACY_OFFSET = timedelta(hours=2)


    def now():
        """Naive local time of the original deployment (UTC+2). Deprecated."""
        warnings.warn("legacy_clock.now() is deprecated; use platform_api.clock.utc_now()",
                      DeprecationWarning, stacklevel=2)
        return (utc_now() + LEGACY_OFFSET).replace(tzinfo=None)
'''

def m5_golden(files: dict[str, str], targets: dict[str, list[str]]) -> dict[str, dict[str, str]]:
    script = (
        "import importlib, json, warnings\n"
        "from datetime import datetime, timezone\n"
        "from platform_api import clock\n"
        "clock.freeze(datetime(2026, 3, 14, 23, 30, tzinfo=timezone.utc))\n"
        "warnings.simplefilter('ignore')\n"
        f"TARGETS = {targets!r}\n"
        "result = {}\n"
        "for name, exprs in TARGETS.items():\n"
        "    module = importlib.import_module(name)\n"
        "    result[name] = {}\n"
        "    for expr in exprs:\n"
        "        try:\n"
        "            result[name][expr] = repr(eval(expr, vars(module)))\n"
        "        except Exception as error:\n"
        "            result[name][expr] = 'raises ' + type(error).__name__\n"
        "print(json.dumps(result))\n")
    return run_golden(files, script)


M5_HIDDEN = r'''
import ast
import hashlib
import importlib
import pathlib
import warnings
from datetime import datetime, timezone

import pytest

import legacy_clock
import legacy_money
from platform_api import clock

ROOT = pathlib.Path(__file__).resolve().parent
FROZEN = datetime(2026, 3, 14, 23, 30, tzinfo=timezone.utc)
MIGRATED = @MIGRATED@
DECOYS = @DECOYS@
ALL = @ALL@
SERVICE_GOLDEN = @SERVICE_GOLDEN@
DECOY_SHA256 = @DECOY_SHA256@
VENDOR_SHA256 = @VENDOR_SHA256@
VENDOR_GOLDEN = @VENDOR_GOLDEN@


@pytest.fixture(autouse=True)
def frozen_clock():
    clock.freeze(FROZEN)
    yield
    clock.freeze(None)


def service(name):
    return importlib.import_module("services." + name)


def source(name):
    return (ROOT / "services" / (name + ".py")).read_text(encoding="utf-8")


# Constraint 4: header comment on every migrated module (and only those).
@pytest.mark.parametrize("name", MIGRATED)
def test_c4_header_on_migrated_module(name):
    assert source(name).split("\n", 1)[0] == "# api: v2"


@pytest.mark.parametrize("name", DECOYS)
def test_c4_unrelated_module_is_byte_identical(name):
    data = (ROOT / "services" / (name + ".py")).read_bytes()
    assert hashlib.sha256(data).hexdigest() == DECOY_SHA256[name]


# Constraint 3: __all__ unchanged.
@pytest.mark.parametrize("name", MIGRATED + DECOYS)
def test_c3_all_is_unchanged(name):
    assert list(service(name).__all__) == ALL[name]


# The migration itself: v2 semantics, no use of the old API.
@pytest.mark.parametrize("name", MIGRATED)
def test_migrated_module_has_no_legacy_references(name):
    tree = ast.parse(source(name))
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            assert not any(a.name.startswith("legacy_") for a in node.names), name
        if isinstance(node, ast.ImportFrom):
            assert not (node.module or "").startswith("legacy_"), name
        if isinstance(node, ast.Name):
            assert node.id != "fmt_money", name
        if isinstance(node, ast.Attribute):
            assert node.attr != "fmt_money", name


@pytest.mark.parametrize("name", MIGRATED)
def test_migrated_module_matches_v2_semantics(name):
    module = service(name)
    expected = SERVICE_GOLDEN[name]
    with warnings.catch_warnings():
        warnings.simplefilter("error", DeprecationWarning)
        for expr, value in expected.items():
            try:
                actual = repr(eval(expr, vars(module)))
            except Exception as error:
                actual = "raises " + type(error).__name__
            assert actual == value, (name, expr)


@pytest.mark.parametrize("name", DECOYS)
def test_unrelated_module_behaviour_is_unchanged(name):
    module = service(name)
    for expr, value in SERVICE_GOLDEN[name].items():
        assert repr(eval(expr, vars(module))) == value, (name, expr)


def test_all_service_files_are_still_present():
    present = sorted(p.stem for p in (ROOT / "services").glob("*.py") if p.stem != "__init__")
    assert present == sorted(MIGRATED + DECOYS)


# Constraint 1: legacy_clock stays as a deprecating shim with the same value.
def test_c1_legacy_clock_shim_warns_and_keeps_its_value():
    with pytest.warns(DeprecationWarning):
        value = legacy_clock.now()
    assert value == datetime(2026, 3, 15, 1, 30)
    assert value.tzinfo is None


def test_c1_shim_warning_points_at_the_caller():
    with pytest.warns(DeprecationWarning) as caught:
        legacy_clock.now()
    assert caught[0].filename == __file__


def test_legacy_money_still_formats_the_old_way():
    assert legacy_money.fmt_money(123456) == "$1,234.56"
    assert legacy_money.fmt_money(-5) == "-$0.05"


# Constraint 2: vendor/ is untouched and keeps working.
def test_c2_vendor_files_are_unmodified():
    found = {p.relative_to(ROOT).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest()
             for p in sorted((ROOT / "vendor").rglob("*.py"))}
    assert found == VENDOR_SHA256


def test_c2_vendor_code_still_works_through_the_shim():
    for name, exprs in VENDOR_GOLDEN.items():
        module = importlib.import_module(name)
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", DeprecationWarning)
            for expr, value in exprs.items():
                assert repr(eval(expr, vars(module))) == value, (name, expr)
'''

M5_PUBLIC = '''
    import warnings
    from datetime import datetime, timezone

    import pytest

    import legacy_clock
    from platform_api import clock
    from platform_api.money import format_money
    from services import @FIRST@


    @pytest.fixture(autouse=True)
    def frozen_clock():
        clock.freeze(datetime(2026, 3, 14, 23, 30, tzinfo=timezone.utc))
        yield
        clock.freeze(None)


    def test_v2_clock_is_aware_utc():
        assert clock.utc_now().utcoffset().total_seconds() == 0


    def test_v2_money_takes_a_currency():
        assert format_money(1234567, "EUR") == "12,345.67 EUR"
        with pytest.raises(ValueError):
            format_money(1, "XXX")


    def test_legacy_clock_still_returns_the_old_local_time():
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", DeprecationWarning)
            assert legacy_clock.now() == datetime(2026, 3, 15, 1, 30)


    def test_pure_service_helpers_do_not_change():
        assert @FIRST@.@FIRST@_tax(10050, @REGION@) == @TAX@
        assert @FIRST@.@FIRST@_tiers(37, 1999) == @TIERS@
'''


def build_m5() -> dict[str, object]:
    starter, solution, targets_new, targets_decoy, targets_legacy = {}, {}, {}, {}, {}
    meta = []
    for index in range(len(M5_DOMAINS)):
        old, info = m5_service(index, False)
        new, _ = m5_service(index, True)
        name = info["name"]
        starter[f"services/{name}.py"] = old
        solution[f"services/{name}.py"] = new
        meta.append(info)
    support = dict(M5_PLATFORM)
    base_files = {"legacy_clock.py": textwrap.dedent(M5_LEGACY_CLOCK).lstrip("\n"), "services/__init__.py": ""}
    for relative, value in support.items():
        base_files[relative] = textwrap.dedent(value).lstrip("\n")
    vendor = {"vendor/ledgerkit/__init__.py": '"""Vendored ledgerkit (upstream copy)."""\n'}
    for part in range(6):
        vendor[f"vendor/ledgerkit/part_{part}.py"] = textwrap.dedent(m5_vendor(part)).lstrip("\n")
    vendor_targets = {}
    for part in range(6):
        k = part
        vendor_targets[f"vendor.ledgerkit.part_{k}"] = [
            f"stamp_{k}('v-')", f"window_{k}(5)", f"label_{k}(12345)", f"spread_{k}([1, 250, -3])",
            f"checksum_{k}('ledger')"]
    legacy_files = {**base_files, **vendor, **{k: textwrap.dedent(v).lstrip("\n") for k, v in starter.items()}}
    vendor_golden = m5_golden(legacy_files, vendor_targets)
    reference_files = {**legacy_files, **{k: textwrap.dedent(v).lstrip("\n") for k, v in solution.items()},
                       "legacy_clock.py": textwrap.dedent(M5_LEGACY_CLOCK_SHIM).lstrip("\n")}
    targets = {f"services.{m['name']}": m["smoke"] for m in meta}
    reference_golden = m5_golden(reference_files, targets)
    starter_golden = m5_golden(legacy_files, targets)
    migrated = [m["name"] for m in meta if not m["decoy"]]
    decoys = [m["name"] for m in meta if m["decoy"]]
    for name in migrated:
        # The grader must be able to tell the untouched service from the migrated one.
        if starter_golden[f"services.{name}"] == reference_golden[f"services.{name}"]:
            raise ValueError("migration is invisible for " + name)
    for name in decoys:
        if starter_golden[f"services.{name}"] != reference_golden[f"services.{name}"]:
            raise ValueError("decoy changed: " + name)
    hidden_text = fill(
        M5_HIDDEN.lstrip("\n"),
        MIGRATED=literal(migrated), DECOYS=literal(decoys),
        ALL=literal({m["name"]: m["all"] for m in meta}),
        SERVICE_GOLDEN=literal({m["name"]: reference_golden[f"services.{m['name']}"] for m in meta}),
        DECOY_SHA256=literal({n: hashlib.sha256(legacy_files[f"services/{n}.py"].encode("utf-8")).hexdigest()
                               for n in decoys}),
        VENDOR_SHA256=literal({k: hashlib.sha256(v.encode("utf-8")).hexdigest() for k, v in vendor.items()}),
        VENDOR_GOLDEN=literal(vendor_golden))
    first = meta[0]["name"]
    first_gold = reference_golden[f"services.{first}"]
    tax_expr = next(e for e in first_gold if e.startswith(f"{first}_tax("))
    region = tax_expr.split(", ", 1)[1].rstrip(")")
    public = fill(M5_PUBLIC, FIRST=first, REGION=region, TAX=first_gold[tax_expr],
                  TIERS=first_gold[f"{first}_tiers(37, 1999)"])
    repo = {**{k: v for k, v in base_files.items()}, **vendor, **starter,
            "README.md": (
                "# Platform services\n\nServices live in `services/`. `platform_api` is the v2 API "
                "(`clock.utc_now()` returns aware UTC time; `money.format_money(cents, currency)`). "
                "`legacy_clock` and `legacy_money` are the old helpers; `vendor/` is an upstream copy.\n"),
            "tests/test_public_services.py": public}
    allowed = ([f"services/{m['name']}.py" for m in meta] + ["legacy_clock.py", "tests/test_public_services.py"])
    return {
        "id": "m5-wide-migration",
        "prompt": (
            "Constraints (they apply for the whole task, including the last file you touch):\n"
            "1. Keep legacy_clock.py as a compatibility shim: legacy_clock.now() must return exactly "
            "what it returns today and emit a DeprecationWarning (stacklevel so the warning points at "
            "the caller).\n"
            "2. Never modify anything under vendor/. Vendored code also calls the old API and must "
            "keep working through the shim.\n"
            "3. Keep every module's __all__ exactly as it is.\n"
            "4. Every module you migrate gets the one-line header comment `# api: v2` as its first line. "
            "Do not touch modules that do not use the old API; they must stay byte-identical.\n\n"
            "Task: migrate every module under services/ that calls legacy_clock.now() or "
            "legacy_money.fmt_money(cents) to the v2 platform API. platform_api.clock.utc_now() "
            "returns timezone-aware UTC time (the old now() returned naive UTC+2 local time) and "
            "platform_api.money.format_money(cents, currency) takes a currency, which is the "
            "module's own reporting currency constant. Migrated modules must not use the old API "
            "in any form, including aliases, module-qualified calls, lazy imports and passing "
            "fmt_money as a function value. Finish by running "
            "python -m pytest -q tests/test_public_services.py."),
        "allowed": allowed,
        "public_verification": "python -m pytest -q tests/test_public_services.py",
        "repo": repo,
        "hidden": {"test_hidden_migration.py": hidden_text},
        "solution": {**solution, "legacy_clock.py": M5_LEGACY_CLOCK_SHIM},
    }


TASK_SPECS.append(build_m5())


# ---------------------------------------------------------------------------
# m6-requirements-ledger
# ---------------------------------------------------------------------------

M6_CATEGORY_NAMES = [
    "fasteners", "adhesives", "abrasives", "bearings", "belts", "brackets", "cables", "castors",
    "clamps", "connectors", "coolants", "couplings", "drills", "dyes", "enclosures", "fans",
    "filters", "fittings", "gaskets", "gears", "gloves", "hinges", "hoses", "inks", "lamps",
    "lubricants", "magnets", "meters", "nozzles", "paints", "pumps", "relays", "seals",
    "sensors", "solvents", "springs", "switches", "tapes", "valves", "wipes",
]
M6_TAX_CLASSES = ["standard", "standard", "reduced", "books", "zero"]
M6_WORDS = ["steel", "brass", "nylon", "copper", "zinc", "rubber", "alloy", "ceramic", "carbon", "teflon",
            "heavy", "light", "fine", "coarse", "sealed", "vented", "flat", "round", "hex", "slotted"]
M6_REASONS = ("receipt", "sale", "return", "damage", "correction", "transfer")


def m6_categories_source(fixed: bool) -> str:
    out = ['"""Category rules: pack sizes, lead times, tax class, backorder policy and volume tiers."""\n\n'
           + ('from warehouse.errors import UnknownCategoryError\n\n' if fixed else '\n') +
           '__all__ = ["CATEGORY_RULES", "rule_for", "tiers_for", "pack_size_for", "lead_days_for", '
           '"is_backorderable"]\n\n'
           'CATEGORY_RULES = {\n']
    for index, name in enumerate(M6_CATEGORY_NAMES):
        rng = Rng(300 + index)
        tiers, quantity = [], 0
        for step in range(rng.span(2, 4)):
            quantity += rng.span(8, 60)
            tiers.append((quantity, 150 + step * rng.span(120, 400) + rng.span(0, 90)))
        out.append(
            f'    "{name}": {{\n'
            f'        "pack_size": {rng.pick([1, 2, 5, 6, 10, 12, 24, 25, 50])},\n'
            f'        "lead_days": {rng.span(3, 28)},\n'
            f'        "tax_class": "{M6_TAX_CLASSES[index % 5]}",\n'
            f'        "backorderable": {index % 3 == 0},\n'
            f'        "tiers": {tiers!r},\n'
            f'        "notes": "{rng.pick(M6_WORDS)} {name}, {rng.pick(M6_WORDS)} grade, '
            f'ships from bay {rng.span(1, 40)}",\n'
            f'    }},\n')
    out.append('}\n\n\n')
    if fixed:
        out.append('''def rule_for(category):
    """Rules of one category."""
    try:
        return CATEGORY_RULES[category]
    except (KeyError, TypeError):
        raise UnknownCategoryError(f"unknown category: {category!r}") from None
''')
    else:
        out.append('''def rule_for(category):
    """Rules of one category."""
    return CATEGORY_RULES[category]
''')
    out.append('''

def tiers_for(category):
    """Volume tiers as (minimum quantity, discount in basis points), ascending."""
    return list(rule_for(category)["tiers"])


def pack_size_for(category):
    return rule_for(category)["pack_size"]


def lead_days_for(category):
    return rule_for(category)["lead_days"]


def is_backorderable(category):
    return bool(rule_for(category)["backorderable"])
''')
    return "".join(out)


M6_PRODUCT_COUNT = 300
M6_REGION_COUNT = 120


def m6_catalog_source() -> str:
    rng = Rng(77)
    rows = []
    for number in range(1, M6_PRODUCT_COUNT + 1):
        category = M6_CATEGORY_NAMES[(number * 7 + rng.below(3)) % len(M6_CATEGORY_NAMES)]
        name = f"{rng.pick(M6_WORDS).capitalize()} {category[:-1]} {rng.span(2, 90)}{rng.pick(['mm', 'cm', 'g', 'ml'])}"
        rows.append((f"SKU-{number:04d}", name, category, rng.span(80, 45000), rng.span(5, 2400)))
    body = "".join(f"    {row!r},\n" for row in rows)
    return ('"""Product catalog seed. Rows: sku, name, category, unit_cents, weight_g."""\n\n'
            'PRODUCTS = [\n' + body + ']\n')


def m6_regions_source(fixed: bool) -> str:
    out = ['"""Sales tax by region: rate in basis points and the tax classes the region exempts."""\n\n'
           + ('from warehouse.errors import UnknownRegionError\n\n' if fixed else '\n') +
           '__all__ = ["REGIONS", "tax_rate"]\n\nREGIONS = {\n']
    for index in range(M6_REGION_COUNT):
        rng = Rng(900 + index)
        code = f"{rng.pick(['US', 'CA', 'DE', 'FR', 'NL', 'ES', 'IT', 'SE'])}-{index:03d}"
        exempt = [("books",), ("books", "zero"), (), ("zero",)][index % 4]
        out.append(f'    "{code}": {{"rate_bps": {rng.span(0, 2500)}, "exempt": {exempt!r}}},'
                   f'  # {rng.pick(M6_WORDS)} district {rng.span(1, 99)}\n')
    out.append("}\n\n\n")
    if fixed:
        out.append('''def tax_rate(region, tax_class):
    """Tax rate in basis points for `tax_class` sold into `region`; exempt classes pay none."""
    try:
        entry = REGIONS[region]
    except (KeyError, TypeError):
        raise UnknownRegionError(f"unknown region: {region!r}") from None
    if tax_class in entry["exempt"]:
        return 0
    return entry["rate_bps"]
''')
    else:
        out.append('''def tax_rate(region, tax_class):
    """Tax rate in basis points for `tax_class` sold into `region`."""
    if region not in REGIONS:
        raise ValueError(f"unknown region: {region!r}")
    return REGIONS[region]["rate_bps"]
''')
    return "".join(out)


M6_SUPPLIERS = [
    "acme", "borealis", "cobalt", "delta", "ember", "falcon", "granite", "harbor", "ivory", "juniper",
    "kestrel", "lumen", "meridian", "nimbus", "onyx", "pioneer", "quartz", "raven", "summit", "tundra",
    "umber", "vertex", "willow", "xenon", "yonder", "zenith", "atlas", "bluff", "cedar", "dune",
]


def _more_suppliers() -> list[str]:
    rng = Rng(31337)
    heads = ["al", "bra", "cor", "dra", "el", "fen", "gal", "hel", "in", "jor", "kal", "lor", "mir", "nor",
             "ol", "par", "quin", "ros", "sel", "tor", "ul", "val", "wen", "xan", "yor", "zel"]
    tails = ["ba", "dex", "fin", "gon", "hart", "ley", "mont", "nex", "pool", "rick", "son", "ton", "vale", "wick"]
    names = set(M6_SUPPLIERS)
    result = list(M6_SUPPLIERS)
    while len(result) < 80:
        name = rng.pick(heads) + rng.pick(tails)
        if name not in names:
            names.add(name)
            result.append(name)
    return result


M6_SUPPLIERS = _more_suppliers()


def m6_suppliers_source(fixed: bool) -> tuple[str, dict[str, dict[str, object]]]:
    error = "FeedFormatError" if fixed else "ValueError"
    out = ['"""Supplier feed adapters. One parser per supplier; each returns sku, qty and unit_cents."""\n\n'
           + ('from warehouse.errors import FeedFormatError\n\n' if fixed else '\n')]
    info: dict[str, dict[str, object]] = {}
    for index, name in enumerate(M6_SUPPLIERS):
        rng = Rng(1700 + index)
        family = index % 3
        delim = rng.pick(["|", ";", "\t", ":", "~"])
        delim_code = delim.replace("\t", "\\t")
        price_mode = rng.pick(["dollars", "cents", "symbol"])
        suffix = rng.pick(["", "", "pc", "ct", "x"])
        prefix = rng.pick(["", "SKU-", "SKU-"])
        order = ["sku", "qty", "price"]
        for position in range(2, 0, -1):
            swap = rng.below(position + 1)
            order[position], order[swap] = order[swap], order[position]
        sku_check = f' or not sku.startswith("{prefix}")' if prefix else ""
        qty_lines = (f'    qty = qty.strip()\n    if qty.endswith("{suffix}"):\n'
                     f'        qty = qty[: -len("{suffix}")]\n') if suffix else "    qty = qty.strip()\n"
        if price_mode == "cents":
            price_lines = (f'    try:\n        cents = int(price)\n    except ValueError:\n'
                           f'        raise {error}(f"{name}: bad price {{price!r}}")\n')
        else:
            strip = '    price = price.strip().lstrip("$")\n' if price_mode == "symbol" else ""
            price_lines = (f'{strip}    try:\n        cents = int(round(float(price) * 100))\n    except ValueError:\n'
                           f'        raise {error}(f"{name}: bad price {{price!r}}")\n')
        good_price = {"dollars": "12.50", "cents": "1250", "symbol": "$12.50"}[price_mode]
        good_qty = "12" + suffix
        sku = f"{prefix}{index + 1:04d}" if prefix else f"X{index + 1:04d}"
        if family == 0:
            fields = {"sku": sku.lower() if index % 2 else sku, "qty": good_qty, "price": good_price}
            line = delim.join(fields[key] for key in order)
            out.append(
                f'def parse_feed_{name}(line):\n'
                f'    """{name.capitalize()} feed: {delim_code!r}-separated {", ".join(order)}; '
                f'prices as {price_mode}; skus {"are prefixed " + prefix if prefix else "are bare"}."""\n'
                f'    text = line.strip()\n    if not text:\n        raise {error}("{name}: empty line")\n'
                f'    parts = text.split("{delim_code}")\n    if len(parts) != 3:\n'
                f'        raise {error}(f"{name}: expected 3 fields, got {{len(parts)}}")\n'
                f'    {", ".join(order)} = parts\n    sku = sku.strip().upper()\n'
                f'    if not sku{sku_check}:\n        raise {error}(f"{name}: bad sku {{sku!r}}")\n'
                f'{qty_lines}    try:\n        quantity = int(qty)\n    except ValueError:\n'
                f'        raise {error}(f"{name}: bad quantity {{qty!r}}")\n'
                f'    if quantity < 0:\n        raise {error}("{name}: negative quantity")\n'
                f'{price_lines}    return {{"sku": sku, "qty": quantity, "unit_cents": cents}}\n\n\n')
            info[name] = {"family": family, "good": line, "bad_unit": None}
        elif family == 1:
            w1, w2, w3 = rng.span(8, 12), rng.span(4, 6), rng.span(6, 8)
            total = w1 + w2 + w3
            line = sku.ljust(w1) + "12".rjust(w2) + "1250".rjust(w3)
            out.append(
                f'def parse_feed_{name}(line):\n'
                f'    """{name.capitalize()} feed: fixed width, sku {w1}, quantity {w2}, price in cents {w3}."""\n'
                f'    text = line.rstrip("\\r\\n")\n    if len(text) < {total}:\n'
                f'        raise {error}(f"{name}: line too short ({{len(text)}} < {total})")\n'
                f'    sku = text[:{w1}].strip().upper()\n    if not sku:\n'
                f'        raise {error}("{name}: missing sku")\n'
                f'    qty = text[{w1}:{w1 + w2}].strip()\n    price = text[{w1 + w2}:{total}].strip()\n'
                f'    try:\n        quantity = int(qty)\n    except ValueError:\n'
                f'        raise {error}(f"{name}: bad quantity {{qty!r}}")\n'
                f'    try:\n        cents = int(price)\n    except ValueError:\n'
                f'        raise {error}(f"{name}: bad price {{price!r}}")\n'
                f'    return {{"sku": sku, "qty": quantity, "unit_cents": cents}}\n\n\n')
            info[name] = {"family": family, "good": line, "bad_unit": None}
        else:
            units = {"EA": 1, "CS": rng.pick([6, 12, 24]), "PK": rng.pick([2, 4, 5]), "GR": 144}
            unit_lookup = (f'    try:\n        factor = UNITS_{name.upper()}[unit.strip().upper()]\n'
                           f'    except KeyError:\n        raise {error}(f"{name}: unknown unit {{unit!r}}") from None\n'
                           if fixed else f'    factor = UNITS_{name.upper()}[unit.strip().upper()]\n')
            line = delim.join([sku, "3", "cs", "12.50"])
            out.append(
                f'UNITS_{name.upper()} = {units!r}\n\n\n'
                f'def parse_feed_{name}(line):\n'
                f'    """{name.capitalize()} feed: sku{delim_code}quantity{delim_code}unit{delim_code}price; '
                f'quantity is counted in units (EA, CS, PK, GR)."""\n'
                f'    text = line.strip()\n    parts = text.split("{delim_code}")\n    if len(parts) != 4:\n'
                f'        raise {error}(f"{name}: expected 4 fields, got {{len(parts)}}")\n'
                f'    sku, qty, unit, price = parts\n    sku = sku.strip().upper()\n    if not sku:\n'
                f'        raise {error}("{name}: missing sku")\n'
                f'{unit_lookup}    try:\n        quantity = int(qty) * factor\n    except ValueError:\n'
                f'        raise {error}(f"{name}: bad quantity {{qty!r}}")\n'
                f'    try:\n        cents = int(round(float(price) * 100))\n    except ValueError:\n'
                f'        raise {error}(f"{name}: bad price {{price!r}}")\n'
                f'    return {{"sku": sku, "qty": quantity, "unit_cents": cents}}\n\n\n')
            info[name] = {"family": family, "good": line, "bad_unit": delim.join([sku, "3", "ZZ", "12.50"])}
    out.append("PARSERS = {\n" + "".join(f'    "{n}": parse_feed_{n},\n' for n in M6_SUPPLIERS) + "}\n\n\n")
    if fixed:
        out.append('def parse_feed(supplier, line):\n    """Parse one line with the adapter of `supplier`."""\n'
                   '    try:\n        parser = PARSERS[supplier]\n    except (KeyError, TypeError):\n'
                   '        raise FeedFormatError(f"unknown supplier: {supplier!r}") from None\n'
                   '    if not isinstance(line, str):\n        raise FeedFormatError("feed line must be text")\n'
                   '    return parser(line)\n\n\n')
    else:
        out.append('def parse_feed(supplier, line):\n    """Parse one line with the adapter of `supplier`."""\n'
                   '    return PARSERS[supplier](line)\n\n\n')
    out.append("__all__ = [\"PARSERS\", \"parse_feed\"] + sorted(f\"parse_feed_{n}\" for n in PARSERS)\n")
    return "".join(out), info


M6_ERRORS_START = '''
    """Errors raised by the warehouse service."""


    class InventoryError(Exception):
        """Base class of every error the public functions are meant to raise."""


    class UnknownSkuError(InventoryError):
        """The sku is not in the catalog."""


    class InsufficientStockError(InventoryError):
        """Not enough stock to satisfy the request."""


    class OrderStateError(InventoryError):
        """The order is in a state that does not allow the operation."""
'''

M6_ERRORS_FIXED = M6_ERRORS_START + '''

    class ValidationError(InventoryError):
        """A value is missing, malformed or out of range."""


    class DuplicateProductError(ValidationError):
        """The sku is already in the catalog."""


    class UnknownCategoryError(InventoryError):
        """The category has no rules."""


    class UnknownRegionError(InventoryError):
        """The region has no tax entry."""


    class UnknownReasonError(ValidationError):
        """The stock adjustment reason is not one of the allowed codes."""


    class UnknownOrderError(InventoryError):
        """The order id is not known."""


    class FeedFormatError(InventoryError):
        """A supplier feed line does not match the supplier's format."""


    class ImportFormatError(InventoryError):
        """The stock file as a whole cannot be read."""
'''

M6_TIMEUTIL_START = '''
    """The service clock. Tests move it with set_now()."""
    from datetime import datetime

    _override = None


    def set_now(value):
        global _override
        _override = value


    def now():
        if _override is not None:
            return _override
        return datetime.utcnow()


    def iso(value):
        return value.isoformat()


    def stamp():
        return iso(now())
'''

M6_TIMEUTIL_FIXED = '''
    """The service clock. Tests move it with set_now()."""
    from datetime import datetime, timezone

    _override = None
    FORMAT = "%Y-%m-%dT%H:%M:%SZ"


    def set_now(value):
        global _override
        _override = value


    def now():
        if _override is not None:
            return _override
        return datetime.now(timezone.utc)


    def iso(value):
        """UTC, whole seconds, trailing Z. Naive values are taken as UTC."""
        if value.tzinfo is not None:
            value = value.astimezone(timezone.utc)
        return value.replace(tzinfo=None, microsecond=0).strftime(FORMAT)


    def parse(text):
        return datetime.strptime(text, FORMAT)


    def stamp():
        return iso(now())
'''

M6_INVENTORY_START = '''
    """Stock levels, reservations and the event log."""
    from warehouse import timeutil
    from warehouse.errors import InsufficientStockError, UnknownSkuError

    __all__ = ["Inventory", "ALLOWED_REASONS"]

    ALLOWED_REASONS = ("receipt", "sale", "return", "damage", "correction", "transfer")


    class Inventory:
        def __init__(self):
            self.products = {}
            self.stock = {}
            self.reservations = {}
            self.events = []

        def add_product(self, sku, name, category, unit_cents):
            if not sku:
                raise ValueError("sku is required")
            if sku in self.products:
                raise ValueError(f"duplicate sku {sku}")
            self.products[sku] = {"sku": sku, "name": name, "category": category, "unit_cents": unit_cents}
            self.stock[sku] = {}
            return self.products[sku]

        def product(self, sku):
            return self.products[sku]

        def adjust_stock(self, sku, delta, reason, warehouse="main"):
            if sku not in self.products:
                raise KeyError(sku)
            if not isinstance(delta, int):
                raise ValueError("delta must be an integer")
            level = self.stock[sku].get(warehouse, 0) + delta
            if level < 0:
                raise ValueError("stock cannot go negative")
            self.stock[sku][warehouse] = level
            self.events.append({"type": "adjust", "sku": sku, "delta": delta, "reason": reason,
                                "warehouse": warehouse, "at": timeutil.stamp()})
            return level

        def on_hand(self, sku, warehouse=None):
            levels = self.stock[sku]
            if warehouse is not None:
                return levels.get(warehouse, 0)
            return sum(levels.values())

        def reserved(self, sku):
            return sum(lines.get(sku, 0) for lines in self.reservations.values())

        def available(self, sku):
            return self.on_hand(sku) - self.reserved(sku)

        def reserve(self, order_id, lines):
            for sku, qty in lines:
                if self.available(sku) < qty:
                    raise InsufficientStockError(f"{sku}: only {self.available(sku)} available")
                held = self.reservations.setdefault(order_id, {})
                held[sku] = held.get(sku, 0) + qty

        def release(self, order_id):
            return self.reservations.pop(order_id)

        def commit(self, order_id):
            """Turn the reservation of a shipped order into a sale."""
            for sku, qty in sorted(self.reservations.pop(order_id).items()):
                for warehouse in sorted(self.stock[sku]):
                    take = min(qty, self.stock[sku][warehouse])
                    if take:
                        self.adjust_stock(sku, -take, "sale", warehouse)
                        qty -= take

        def events_for(self, sku=None):
            return [e for e in self.events if sku is None or e["sku"] == sku]
'''

M6_INVENTORY_FIXED = '''
    """Stock levels, reservations and the event log."""
    from warehouse import timeutil
    from warehouse.errors import (DuplicateProductError, InsufficientStockError, UnknownReasonError,
                                  UnknownSkuError, ValidationError)

    __all__ = ["Inventory", "ALLOWED_REASONS"]

    ALLOWED_REASONS = ("receipt", "sale", "return", "damage", "correction", "transfer")


    class Inventory:
        def __init__(self):
            self.products = {}
            self.stock = {}
            self.reservations = {}
            self.events = []

        def add_product(self, sku, name, category, unit_cents):
            if not isinstance(sku, str) or not sku.strip():
                raise ValidationError("sku is required")
            if sku in self.products:
                raise DuplicateProductError(f"duplicate sku {sku}")
            if isinstance(unit_cents, str):
                try:
                    unit_cents = int(unit_cents.strip())
                except ValueError:
                    raise ValidationError(f"unit_cents is not a number: {unit_cents!r}") from None
            if isinstance(unit_cents, bool) or not isinstance(unit_cents, int) or unit_cents < 0:
                raise ValidationError("unit_cents must be a nonnegative integer")
            self.products[sku] = {"sku": sku, "name": name, "category": category, "unit_cents": unit_cents}
            self.stock[sku] = {}
            return self.products[sku]

        def product(self, sku):
            try:
                return self.products[sku]
            except (KeyError, TypeError):
                raise UnknownSkuError(f"unknown sku: {sku!r}") from None

        def adjust_stock(self, sku, delta, reason, warehouse="main"):
            self.product(sku)
            if reason not in ALLOWED_REASONS:
                raise UnknownReasonError(f"unknown reason: {reason!r}")
            if isinstance(delta, bool) or not isinstance(delta, int):
                raise ValidationError("delta must be an integer")
            level = self.stock[sku].get(warehouse, 0) + delta
            if level < 0:
                raise ValidationError("stock cannot go negative")
            self.stock[sku][warehouse] = level
            self.events.append({"type": "adjust", "sku": sku, "delta": delta, "reason": reason,
                                "warehouse": warehouse, "at": timeutil.stamp()})
            return level

        def on_hand(self, sku, warehouse=None):
            self.product(sku)
            levels = self.stock[sku]
            if warehouse is not None:
                return levels.get(warehouse, 0)
            return sum(levels.values())

        def reserved(self, sku):
            return sum(lines.get(sku, 0) for lines in self.reservations.values())

        def available(self, sku):
            return self.on_hand(sku) - self.reserved(sku)

        def reserve(self, order_id, lines):
            """All lines or none: a short line reserves nothing."""
            wanted = {}
            for sku, qty in lines:
                self.product(sku)
                if isinstance(qty, bool) or not isinstance(qty, int) or qty <= 0:
                    raise ValidationError("reservation quantity must be a positive integer")
                wanted[sku] = wanted.get(sku, 0) + qty
            for sku, qty in wanted.items():
                if self.available(sku) < qty:
                    raise InsufficientStockError(f"{sku}: only {self.available(sku)} available")
            held = self.reservations.setdefault(order_id, {})
            for sku, qty in wanted.items():
                held[sku] = held.get(sku, 0) + qty

        def release(self, order_id):
            return self.reservations.pop(order_id, {})

        def commit(self, order_id):
            """Turn the reservation of a shipped order into a sale."""
            for sku, qty in sorted(self.reservations.pop(order_id, {}).items()):
                for warehouse in sorted(self.stock[sku]):
                    take = min(qty, self.stock[sku][warehouse])
                    if take:
                        self.adjust_stock(sku, -take, "sale", warehouse)
                        qty -= take

        def events_for(self, sku=None):
            return [e for e in self.events if sku is None or e["sku"] == sku]
'''

M6_PRICING_START = '''
    """Line and order pricing."""
    __all__ = ["price_line", "price_order"]


    def price_line(inventory, sku, qty, region=None):
        product = inventory.product(sku)
        if qty <= 0:
            raise ValueError("qty must be positive")
        subtotal = product["unit_cents"] * qty
        return {"sku": sku, "qty": qty, "unit_cents": product["unit_cents"], "subtotal_cents": subtotal,
                "discount_cents": 0, "tax_cents": 0, "total_cents": subtotal}


    def price_order(inventory, lines, region=None):
        priced = [price_line(inventory, sku, qty, region) for sku, qty in lines]
        return {"lines": priced, "subtotal_cents": sum(l["subtotal_cents"] for l in priced),
                "tax_cents": 0, "total_cents": sum(l["total_cents"] for l in priced)}
'''

M6_PRICING_FIXED = '''
    """Line and order pricing."""
    from warehouse import categories, regions
    from warehouse.errors import ValidationError

    __all__ = ["price_line", "price_order"]


    def _half_up(numerator, denominator):
        return (numerator * 2 + denominator) // (denominator * 2)


    def price_line(inventory, sku, qty, region=None):
        """Subtotal, volume discount, tax and total of one line, all in cents.

        The discount is the basis points of the largest tier whose minimum the quantity reaches,
        rounded half up. Tax applies to subtotal minus discount at the region's rate for the
        category's tax class, rounded half up per line.
        """
        product = inventory.product(sku)
        if isinstance(qty, bool) or not isinstance(qty, int) or qty <= 0:
            raise ValidationError("qty must be a positive integer")
        rule = categories.rule_for(product["category"])
        subtotal = product["unit_cents"] * qty
        bps = 0
        for minimum, tier_bps in rule["tiers"]:
            if qty >= minimum:
                bps = tier_bps
        discount = _half_up(subtotal * bps, 10000)
        taxable = subtotal - discount
        tax = 0
        if region is not None:
            tax = _half_up(taxable * regions.tax_rate(region, rule["tax_class"]), 10000)
        return {"sku": sku, "qty": qty, "unit_cents": product["unit_cents"], "subtotal_cents": subtotal,
                "discount_cents": discount, "tax_cents": tax, "total_cents": taxable + tax}


    def price_order(inventory, lines, region=None):
        priced = [price_line(inventory, sku, qty, region) for sku, qty in lines]
        return {"lines": priced, "subtotal_cents": sum(l["subtotal_cents"] - l["discount_cents"] for l in priced),
                "tax_cents": sum(l["tax_cents"] for l in priced),
                "total_cents": sum(l["total_cents"] for l in priced)}
'''

M6_ORDERS_START = '''
    """Order placement, cancellation and shipping."""
    from datetime import datetime

    from warehouse import pricing, timeutil
    from warehouse.errors import OrderStateError

    __all__ = ["OrderBook"]


    class OrderBook:
        def __init__(self, inventory):
            self.inventory = inventory
            self.orders = {}
            self._seq = 0

        def place_order(self, client_ref, lines, region=None):
            if not client_ref:
                raise ValueError("client_ref is required")
            if not lines:
                raise ValueError("an order needs at least one line")
            self._seq += 1
            order_id = f"ORD-{self._seq:05d}"
            self.inventory.reserve(order_id, lines)
            priced = pricing.price_order(self.inventory, lines, region)
            order = {"id": order_id, "client_ref": client_ref, "status": "open", "region": region,
                     "lines": [dict(line, reserved_qty=line["qty"], backorder_qty=0) for line in priced["lines"]],
                     "subtotal_cents": priced["subtotal_cents"], "tax_cents": priced["tax_cents"],
                     "total_cents": priced["total_cents"], "created_at": timeutil.stamp(),
                     "shipped_at": None, "cancelled_at": None, "events": []}
            self.orders[order_id] = order
            return order

        def get_order(self, order_id):
            return self.orders[order_id]

        def cancel_order(self, order_id):
            order = self.get_order(order_id)
            order["status"] = "cancelled"
            order["cancelled_at"] = datetime.utcnow().isoformat()
            return order

        def ship_order(self, order_id):
            order = self.get_order(order_id)
            if order["status"] != "open":
                raise OrderStateError(f"order is {order['status']}")
            self.inventory.commit(order_id)
            order["status"] = "shipped"
            order["shipped_at"] = datetime.utcnow().isoformat()
            return order
'''

M6_ORDERS_FIXED = '''
    """Order placement, cancellation and shipping."""
    from warehouse import categories, pricing, timeutil
    from warehouse.errors import (InsufficientStockError, OrderStateError, UnknownOrderError,
                                  ValidationError)

    __all__ = ["OrderBook"]


    class OrderBook:
        def __init__(self, inventory):
            self.inventory = inventory
            self.orders = {}
            self._by_ref = {}
            self._seq = 0

        def place_order(self, client_ref, lines, region=None):
            """Place an order. A repeated client_ref returns the first order unchanged."""
            if not isinstance(client_ref, str) or not client_ref.strip():
                raise ValidationError("client_ref is required")
            if client_ref in self._by_ref:
                return self.orders[self._by_ref[client_ref]]
            if not lines:
                raise ValidationError("an order needs at least one line")
            lines = [(sku, qty) for sku, qty in lines]
            priced = pricing.price_order(self.inventory, lines, region)
            wanted = {}
            for sku, qty in lines:
                wanted[sku] = wanted.get(sku, 0) + qty
            short = {}
            for sku, qty in wanted.items():
                free = max(self.inventory.available(sku), 0)
                if qty > free:
                    category = self.inventory.product(sku)["category"]
                    if not categories.is_backorderable(category):
                        raise InsufficientStockError(f"{sku}: only {free} available, {qty} wanted")
                    short[sku] = qty - free
            self._seq += 1
            order_id = f"ORD-{self._seq:05d}"
            budget = {sku: qty - short.get(sku, 0) for sku, qty in wanted.items()}
            self.inventory.reserve(order_id, [(sku, qty) for sku, qty in budget.items() if qty > 0])
            order_lines = []
            for line in priced["lines"]:
                take = min(line["qty"], budget[line["sku"]])
                budget[line["sku"]] -= take
                order_lines.append(dict(line, reserved_qty=take, backorder_qty=line["qty"] - take))
            order = {"id": order_id, "client_ref": client_ref, "region": region, "lines": order_lines,
                     "status": "backordered" if short else "open",
                     "subtotal_cents": priced["subtotal_cents"], "tax_cents": priced["tax_cents"],
                     "total_cents": priced["total_cents"], "created_at": timeutil.stamp(),
                     "shipped_at": None, "cancelled_at": None, "events": []}
            self.orders[order_id] = order
            self._by_ref[client_ref] = order_id
            return order

        def get_order(self, order_id):
            try:
                return self.orders[order_id]
            except (KeyError, TypeError):
                raise UnknownOrderError(f"unknown order: {order_id!r}") from None

        def cancel_order(self, order_id):
            """Cancel and release the reservation. Shipped orders cannot be cancelled."""
            order = self.get_order(order_id)
            if order["status"] == "shipped":
                raise OrderStateError("a shipped order cannot be cancelled")
            if order["status"] == "cancelled":
                return order
            self.inventory.release(order_id)
            stamp = timeutil.stamp()
            order["status"] = "cancelled"
            order["cancelled_at"] = stamp
            order["events"].append({"type": "order_cancelled", "at": stamp})
            return order

        def ship_order(self, order_id):
            order = self.get_order(order_id)
            if order["status"] != "open":
                raise OrderStateError(f"order is {order['status']}")
            self.inventory.commit(order_id)
            order["status"] = "shipped"
            order["shipped_at"] = timeutil.stamp()
            return order
'''

M6_REPORTS_START = '''
    """Reports and exports."""
    from datetime import datetime

    __all__ = ["report_header", "low_stock_report", "reorder_suggestions", "export_orders_csv"]


    def report_header(title):
        return {"title": title, "generated_at": datetime.now().isoformat()}


    def low_stock_report(inventory, thresholds):
        rows = []
        for sku, product in inventory.products.items():
            limit = thresholds[product["category"]]
            on_hand = inventory.on_hand(sku)
            if on_hand <= limit:
                rows.append({"sku": sku, "category": product["category"], "on_hand": on_hand,
                             "threshold": limit, "deficit": limit - on_hand})
        return rows


    def reorder_suggestions(inventory, days=30):
        """Not implemented yet."""
        return []


    def export_orders_csv(orders, fields=None):
        fields = fields or ["id", "client_ref", "status", "created_at", "total_cents"]
        lines = [",".join(fields)]
        for order in orders:
            lines.append(",".join(str(order.get(field, "")) for field in fields))
        return "\\n".join(lines)
'''

M6_REPORTS_FIXED = '''
    """Reports and exports."""
    import csv
    import io
    from datetime import timezone

    from warehouse import categories, timeutil
    from warehouse.errors import ValidationError

    __all__ = ["report_header", "low_stock_report", "reorder_suggestions", "export_orders_csv"]

    ORDER_FIELDS = ("id", "client_ref", "status", "region", "created_at", "shipped_at", "cancelled_at",
                    "subtotal_cents", "tax_cents", "total_cents")
    DEFAULT_FIELDS = ("id", "client_ref", "status", "created_at", "total_cents")


    def report_header(title):
        return {"title": title, "generated_at": timeutil.stamp()}


    def low_stock_report(inventory, thresholds):
        """Items below their category threshold (`default` when the category has none).

        Sorted by deficit, largest first, then sku.
        """
        rows = []
        for sku, product in inventory.products.items():
            category = product["category"]
            if category in thresholds:
                limit = thresholds[category]
            elif "default" in thresholds:
                limit = thresholds["default"]
            else:
                raise ValidationError(f"no threshold for category {category!r} and no default")
            on_hand = inventory.on_hand(sku)
            if on_hand < limit:
                rows.append({"sku": sku, "category": category, "on_hand": on_hand,
                             "threshold": limit, "deficit": limit - on_hand})
        return sorted(rows, key=lambda row: (-row["deficit"], row["sku"]))


    def reorder_suggestions(inventory, days=30):
        """Suggest reorder quantities from the sales of the last `days` days.

        outflow = units sold (reason "sale", negative delta) in [now - days, now].
        target = ceil(outflow * lead_days / days); need = target - available.
        When need > 0 the suggestion is need rounded up to a multiple of the pack size.
        """
        if isinstance(days, bool) or not isinstance(days, int) or days <= 0:
            raise ValidationError("days must be a positive integer")
        now = timeutil.now()
        if now.tzinfo is not None:
            now = now.astimezone(timezone.utc).replace(tzinfo=None)
        sold = {}
        for event in inventory.events:
            if event["type"] != "adjust" or event["reason"] != "sale" or event["delta"] >= 0:
                continue
            age = now - timeutil.parse(event["at"])
            if 0 <= age.total_seconds() <= days * 86400:
                sold[event["sku"]] = sold.get(event["sku"], 0) - event["delta"]
        suggestions = []
        for sku in sorted(sold):
            category = inventory.product(sku)["category"]
            target = -(-sold[sku] * categories.lead_days_for(category) // days)
            need = target - inventory.available(sku)
            if need > 0:
                pack = categories.pack_size_for(category)
                suggestions.append({"sku": sku, "suggested_qty": -(-need // pack) * pack})
        return suggestions


    def _amount(cents):
        sign = "-" if cents < 0 else ""
        return f"{sign}{abs(cents) // 100}.{abs(cents) % 100:02d}"


    def export_orders_csv(orders, fields=None):
        """CSV with the requested columns in the requested order.

        Fields ending in _cents are written as decimal amounts (1250 -> 12.50) under a header
        without the suffix (total_cents -> total).
        """
        fields = list(fields or DEFAULT_FIELDS)
        for field in fields:
            if field not in ORDER_FIELDS:
                raise ValidationError(f"unknown order field: {field!r}")
        buffer = io.StringIO()
        writer = csv.writer(buffer, lineterminator="\\n")
        writer.writerow([f[: -len("_cents")] if f.endswith("_cents") else f for f in fields])
        for order in orders:
            row = []
            for field in fields:
                value = order.get(field)
                if value is None:
                    row.append("")
                elif field.endswith("_cents"):
                    row.append(_amount(value))
                else:
                    row.append(str(value))
            writer.writerow(row)
        return buffer.getvalue()
'''

M6_IMPORTERS_START = '''
    """Stock file import."""
    __all__ = ["import_stock_csv"]


    def import_stock_csv(inventory, text):
        lines = text.splitlines()
        header = [name.strip().lower() for name in lines[0].split(",")]
        applied = 0
        for line in lines[1:]:
            cells = line.split(",")
            row = dict(zip(header, cells))
            inventory.adjust_stock(row["sku"].strip(), int(row["qty"]), "receipt", row.get("warehouse", "main"))
            applied += 1
        return {"applied": applied}
'''

M6_IMPORTERS_FIXED = '''
    """Stock file import."""
    from dataclasses import dataclass, field

    from warehouse.errors import ImportFormatError, InventoryError

    __all__ = ["import_stock_csv", "ImportReport"]


    @dataclass
    class ImportReport:
        applied: int = 0
        errors: list = field(default_factory=list)


    def _delimiter(header_line):
        counts = {d: header_line.count(d) for d in (",", ";", "\\t")}
        best = max(counts.values())
        return next(d for d in (",", ";", "\\t") if counts[d] == best)


    def import_stock_csv(inventory, text):
        """Apply `sku,qty[,warehouse]` rows as receipts.

        Reads a BOM, picks the delimiter from the header (comma, semicolon or tab), skips blank
        lines and collects bad rows in `errors` with their physical line number (the header is
        line 1) instead of stopping.
        """
        if not isinstance(text, str):
            raise ImportFormatError("stock file must be text")
        lines = text.lstrip("\\ufeff").splitlines()
        if not lines:
            raise ImportFormatError("stock file is empty")
        delimiter = _delimiter(lines[0])
        header = [name.strip().lower() for name in lines[0].split(delimiter)]
        if "sku" not in header or "qty" not in header:
            raise ImportFormatError("header must contain sku and qty")
        report = ImportReport()
        for number, line in enumerate(lines[1:], 2):
            if not line.strip():
                continue
            row = dict(zip(header, (cell.strip() for cell in line.split(delimiter))))
            try:
                qty = int(row.get("qty", ""))
            except ValueError:
                report.errors.append({"row": number, "message": f"bad quantity {row.get('qty')!r}"})
                continue
            try:
                inventory.adjust_stock(row.get("sku", ""), qty, "receipt", row.get("warehouse") or "main")
            except InventoryError as error:
                report.errors.append({"row": number, "message": str(error)})
                continue
            report.applied += 1
        return report
'''

M6_INIT = '''
    """Warehouse inventory and orders."""
    from warehouse.inventory import Inventory
    from warehouse.orders import OrderBook

    __all__ = ["Inventory", "OrderBook"]
'''

M6_PUBLIC = '''
    import json
    from pathlib import Path

    import pytest

    from warehouse import timeutil
    from warehouse.errors import InventoryError
    from warehouse.importers import import_stock_csv
    from warehouse.inventory import Inventory
    from warehouse.orders import OrderBook
    from warehouse.pricing import price_line

    FIXTURES = Path(__file__).resolve().parent / "fixtures"


    def seeded():
        inventory = Inventory()
        for row in json.loads((FIXTURES / "products_seed.json").read_text(encoding="utf-8")):
            inventory.add_product(row["sku"], row["name"], row["category"], row["unit_cents"])
        return inventory


    def test_seed_catalog_loads_every_product():
        assert len(seeded().products) == @COUNT@


    def test_seed_prices_are_stored_as_integer_cents():
        assert all(isinstance(p["unit_cents"], int) for p in seeded().products.values())


    def test_place_order_reserves_stock():
        inventory = seeded()
        inventory.adjust_stock("SKU-0001", 10, "receipt")
        order = OrderBook(inventory).place_order("ref-1", [("SKU-0001", 4)])
        assert order["status"] == "open"
        assert inventory.available("SKU-0001") == 6


    def test_price_line_without_tiers_or_tax_is_the_plain_subtotal():
        inventory = seeded()
        line = price_line(inventory, "SKU-0002", 1)
        assert line["subtotal_cents"] == inventory.product("SKU-0002")["unit_cents"]


    def test_comma_import_applies_rows():
        inventory = seeded()
        text = (FIXTURES / "stock_comma.csv").read_text(encoding="utf-8")
        report = import_stock_csv(inventory, text)
        assert getattr(report, "applied", None) == 5 or report == {"applied": 5}


    def test_unknown_sku_is_an_inventory_error():
        with pytest.raises(InventoryError):
            seeded().adjust_stock("NOPE", 1, "receipt")


    def test_timestamps_come_from_the_service_clock():
        from datetime import datetime

        timeutil.set_now(datetime(2026, 3, 14, 23, 30, 5))
        try:
            assert timeutil.stamp().startswith("2026-03-14T23:30:05")
        finally:
            timeutil.set_now(None)
'''

M6_HIDDEN = r'''
import hashlib
import json
from datetime import datetime, timedelta, timezone
from pathlib import Path

import pytest

from warehouse import categories, regions, reports, suppliers, timeutil
from warehouse.catalog import PRODUCTS
from warehouse.errors import InsufficientStockError, InventoryError
from warehouse.importers import import_stock_csv
from warehouse.inventory import Inventory
from warehouse.orders import OrderBook
from warehouse.pricing import price_line, price_order

ROOT = Path(__file__).resolve().parent
FIXTURE_SHA256 = @FIXTURE_SHA256@
FEED_GOLDEN = @FEED_GOLDEN@
FEED_BAD_UNIT = @FEED_BAD_UNIT@
Z = r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$"


@pytest.fixture(autouse=True)
def clock():
    timeutil.set_now(datetime(2026, 3, 14, 23, 30, 5, tzinfo=timezone(timedelta(hours=2))))
    yield
    timeutil.set_now(None)


def full_inventory():
    inventory = Inventory()
    for sku, name, category, cents, _weight in PRODUCTS:
        inventory.add_product(sku, name, category, cents)
    return inventory


def sku_where(predicate):
    for sku, _name, category, _cents, _weight in PRODUCTS:
        if predicate(categories.CATEGORY_RULES[category]):
            return sku
    raise AssertionError("fixture has no matching product")


def stocked(sku, quantity, inventory=None):
    inventory = inventory or full_inventory()
    inventory.adjust_stock(sku, quantity, "receipt")
    return inventory


def plain_sku():
    return sku_where(lambda rule: not rule["backorderable"])


def backorder_sku():
    return sku_where(lambda rule: rule["backorderable"])


def half_up(numerator, denominator):
    return (numerator * 2 + denominator) // (denominator * 2)


# ---- Requirement 1: public functions raise InventoryError subclasses -------------------------

def _cases():
    sku = plain_sku()
    cases = {
        "product-unknown": lambda: full_inventory().product("NOPE"),
        "adjust-unknown-sku": lambda: full_inventory().adjust_stock("NOPE", 1, "receipt"),
        "adjust-non-integer": lambda: full_inventory().adjust_stock(sku, "x", "receipt"),
        "adjust-negative-level": lambda: full_inventory().adjust_stock(sku, -5, "sale"),
        "add-empty-sku": lambda: Inventory().add_product("", "n", "fasteners", 100),
        "add-duplicate": lambda: full_inventory().add_product(sku, "n", "fasteners", 100),
        "add-bad-cents": lambda: Inventory().add_product("A", "n", "fasteners", "abc"),
        "on-hand-unknown": lambda: full_inventory().on_hand("NOPE"),
        "available-unknown": lambda: full_inventory().available("NOPE"),
        "reserve-unknown": lambda: full_inventory().reserve("O1", [("NOPE", 1)]),
        "reserve-short": lambda: full_inventory().reserve("O1", [(sku, 5)]),
        "price-unknown-sku": lambda: price_line(full_inventory(), "NOPE", 1),
        "price-zero-qty": lambda: price_line(full_inventory(), sku, 0),
        "price-unknown-region": lambda: price_line(full_inventory(), sku, 1, "ZZ-99"),
        "tax-rate-unknown-region": lambda: regions.tax_rate("ZZ-99", "standard"),
        "rule-for-unknown": lambda: categories.rule_for("nope"),
        "tiers-for-unknown": lambda: categories.tiers_for("nope"),
        "place-empty-ref": lambda: OrderBook(stocked(sku, 5)).place_order("", [(sku, 1)]),
        "place-no-lines": lambda: OrderBook(full_inventory()).place_order("r", []),
        "place-unknown-sku": lambda: OrderBook(full_inventory()).place_order("r", [("NOPE", 1)]),
        "cancel-unknown": lambda: OrderBook(full_inventory()).cancel_order("missing"),
        "ship-unknown": lambda: OrderBook(full_inventory()).ship_order("missing"),
        "get-unknown": lambda: OrderBook(full_inventory()).get_order("missing"),
        "import-bad-header": lambda: import_stock_csv(full_inventory(), "a,b,c\n1,2,3\n"),
        "import-not-text": lambda: import_stock_csv(full_inventory(), None),
        "low-stock-no-threshold": lambda: reports.low_stock_report(full_inventory(), {}),
        "reorder-zero-days": lambda: reports.reorder_suggestions(full_inventory(), days=0),
        "export-unknown-field": lambda: reports.export_orders_csv([], ["nope"]),
        "feed-unknown-supplier": lambda: suppliers.parse_feed("nope", "x"),
    }
    for name in sorted(suppliers.PARSERS):
        cases[f"feed-bad-line-{name}"] = (lambda n=name: suppliers.parse_feed(n, "###"))
    for name, line in FEED_BAD_UNIT.items():
        cases[f"feed-bad-unit-{name}"] = (lambda n=name, l=line: suppliers.parse_feed(n, l))
    return cases


CASES = _cases()


@pytest.mark.parametrize("label", sorted(CASES))
def test_req01_raises_inventory_error(label):
    with pytest.raises(Exception) as caught:
        CASES[label]()
    assert isinstance(caught.value, InventoryError), type(caught.value)
    assert type(caught.value) not in (ValueError, KeyError, TypeError, IndexError)


def test_req01_adapters_keep_parsing_good_lines():
    for name, (line, expected) in FEED_GOLDEN.items():
        assert suppliers.parse_feed(name, line) == expected, name


# ---- Requirement 2: tests/fixtures is never edited ----------------------------------------------

def test_req02_fixtures_are_untouched():
    found = {p.relative_to(ROOT).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest()
             for p in sorted((ROOT / "tests" / "fixtures").rglob("*")) if p.is_file()}
    assert found == FIXTURE_SHA256


# ---- Requirement 3: timestamps are ISO-8601 UTC with Z --------------------------------------

def _stamps():
    sku = plain_sku()
    inventory = stocked(sku, 10)
    book = OrderBook(inventory)
    shipped = book.place_order("s", [(sku, 1)])
    book.ship_order(shipped["id"])
    cancelled = book.place_order("c", [(sku, 1)])
    book.cancel_order(cancelled["id"])
    return {"event": inventory.events[0]["at"], "created": shipped["created_at"],
            "shipped": shipped["shipped_at"], "cancelled": cancelled["cancelled_at"],
            "cancel-event": cancelled["events"][0]["at"],
            "header": reports.report_header("x")["generated_at"]}


def test_req03_aware_times_are_converted_to_utc_z():
    for label, value in _stamps().items():
        assert value == "2026-03-14T21:30:05Z", label


def test_req03_naive_times_are_utc_and_drop_microseconds():
    timeutil.set_now(datetime(2026, 3, 14, 23, 30, 5, 123456))
    for label, value in _stamps().items():
        assert value == "2026-03-14T23:30:05Z", label


# ---- Requirement 4: reservations are all or nothing -----------------------------------------

def test_req04_failed_reservation_reserves_nothing():
    first = plain_sku()
    other = next(s for s, _n, c, _p, _w in PRODUCTS if s != first
                 and not categories.CATEGORY_RULES[c]["backorderable"])
    inventory = stocked(first, 5)
    inventory.adjust_stock(other, 1, "receipt")
    with pytest.raises(InsufficientStockError):
        inventory.reserve("O1", [(first, 3), (other, 2)])
    assert inventory.reserved(first) == 0 and inventory.available(first) == 5
    assert inventory.reservations.get("O1", {}) == {}


def test_req04_duplicate_lines_for_one_sku_are_summed():
    sku = plain_sku()
    inventory = stocked(sku, 5)
    with pytest.raises(InsufficientStockError):
        inventory.reserve("O1", [(sku, 3), (sku, 3)])
    assert inventory.reserved(sku) == 0


# ---- Requirement 5: place_order is idempotent per client_ref ---------------------------------

def test_req05_same_client_ref_returns_the_same_order():
    sku = plain_sku()
    book = OrderBook(stocked(sku, 10))
    first = book.place_order("ref-1", [(sku, 2)])
    again = book.place_order("ref-1", [(sku, 7)])
    assert again["id"] == first["id"]
    assert again["lines"][0]["qty"] == 2
    assert len(book.orders) == 1


def test_req05_repeat_does_not_reserve_twice():
    sku = plain_sku()
    inventory = stocked(sku, 10)
    book = OrderBook(inventory)
    book.place_order("ref-1", [(sku, 4)])
    book.place_order("ref-1", [(sku, 4)])
    assert inventory.available(sku) == 6


# ---- Requirement 6: volume tiers ---------------------------------------------------------------

def test_req06_tier_discount_matches_the_category_table():
    sku = plain_sku()
    product = full_inventory().product(sku)
    tiers = categories.tiers_for(product["category"])
    first_minimum = tiers[0][0]
    quantities = [1, first_minimum - 1, first_minimum, tiers[-1][0], tiers[-1][0] + 50]
    for qty in quantities:
        line = price_line(full_inventory(), sku, qty)
        bps = 0
        for minimum, tier_bps in tiers:
            if qty >= minimum:
                bps = tier_bps
        subtotal = product["unit_cents"] * qty
        assert line["subtotal_cents"] == subtotal, qty
        assert line["discount_cents"] == half_up(subtotal * bps, 10000), qty
        assert line["total_cents"] == subtotal - line["discount_cents"], qty


def test_req06_discount_rounds_half_up_on_an_exact_half_cent():
    category, rule = next(iter(categories.CATEGORY_RULES.items()))
    minimum, bps = rule["tiers"][0]
    unit = next(u for u in range(1, 20000) if (u * minimum * bps) % 10000 == 5000)
    inventory = Inventory()
    inventory.add_product("T-1", "tiered", category, unit)
    line = price_line(inventory, "T-1", minimum)
    assert line["discount_cents"] == (unit * minimum * bps) // 10000 + 1


# ---- Requirement 7: backorders ------------------------------------------------------------------

def test_req07_backorderable_shortfall_is_backordered():
    sku = backorder_sku()
    inventory = stocked(sku, 3)
    order = OrderBook(inventory).place_order("b1", [(sku, 5)])
    line = order["lines"][0]
    assert order["status"] == "backordered"
    assert (line["reserved_qty"], line["backorder_qty"]) == (3, 2)
    assert inventory.available(sku) == 0


def test_req07_non_backorderable_shortfall_fails_without_side_effects():
    good, bad = backorder_sku(), plain_sku()
    inventory = stocked(bad, 1)
    inventory.adjust_stock(good, 9, "receipt")
    book = OrderBook(inventory)
    with pytest.raises(InsufficientStockError):
        book.place_order("b2", [(good, 2), (bad, 4)])
    assert inventory.reservations == {}
    assert book.orders == {}


# ---- Requirement 8: cancelling -------------------------------------------------------------------

def test_req08_cancel_releases_reservation_and_logs_an_event():
    sku = plain_sku()
    inventory = stocked(sku, 10)
    book = OrderBook(inventory)
    order = book.place_order("c1", [(sku, 4)])
    book.cancel_order(order["id"])
    assert order["status"] == "cancelled"
    assert inventory.available(sku) == 10
    assert [e["type"] for e in order["events"]] == ["order_cancelled"]


def test_req08_cancelling_a_shipped_order_is_rejected():
    sku = plain_sku()
    book = OrderBook(stocked(sku, 10))
    order = book.place_order("c2", [(sku, 1)])
    book.ship_order(order["id"])
    with pytest.raises(InventoryError):
        book.cancel_order(order["id"])
    assert order["status"] == "shipped"


def test_req08_cancelling_twice_changes_nothing():
    sku = plain_sku()
    book = OrderBook(stocked(sku, 10))
    order = book.place_order("c3", [(sku, 1)])
    book.cancel_order(order["id"])
    book.cancel_order(order["id"])
    assert len(order["events"]) == 1


# ---- Requirement 9: low stock report ---------------------------------------------------------------

def test_req09_low_stock_report_is_sorted_and_uses_the_default():
    inventory = full_inventory()
    a, b, c = (row[0] for row in PRODUCTS[:3])
    inventory.adjust_stock(a, 4, "receipt")
    inventory.adjust_stock(b, 9, "receipt")
    inventory.adjust_stock(c, 10, "receipt")
    rows = reports.low_stock_report(inventory, {"default": 10})
    keys = [(-r["deficit"], r["sku"]) for r in rows]
    assert keys == sorted(keys)
    assert c not in [r["sku"] for r in rows]
    assert [r["sku"] for r in rows][-2:] == [a, b]
    assert rows[-2] == {"sku": a, "category": PRODUCTS[0][2], "on_hand": 4, "threshold": 10, "deficit": 6}


def test_req09_category_threshold_overrides_the_default():
    inventory = full_inventory()
    sku, category = PRODUCTS[0][0], PRODUCTS[0][2]
    inventory.adjust_stock(sku, 5, "receipt")
    rows = reports.low_stock_report(inventory, {"default": 0, category: 6})
    assert [r["threshold"] for r in rows if r["sku"] == sku] == [6]
    assert all(r["category"] == category for r in rows)


# ---- Requirement 10: CSV import ------------------------------------------------------------------------

def test_req10_comma_file_collects_bad_rows_and_applies_good_ones():
    inventory = full_inventory()
    sku_a, sku_b = PRODUCTS[0][0], PRODUCTS[1][0]
    text = f"sku,qty\n{sku_a},5\nNOPE,3\n{sku_b},x\n{sku_b},7\n"
    report = import_stock_csv(inventory, text)
    assert report.applied == 2
    assert [e["row"] for e in report.errors] == [3, 4]
    assert inventory.on_hand(sku_a) == 5 and inventory.on_hand(sku_b) == 7


def test_req10_semicolon_bom_and_blank_lines():
    inventory = full_inventory()
    sku_a, sku_b = PRODUCTS[2][0], PRODUCTS[3][0]
    text = f"\ufeffSKU ; Qty\n{sku_a};4\n\n   \n{sku_b};6\n"
    report = import_stock_csv(inventory, text)
    assert (report.applied, report.errors) == (2, [])
    assert inventory.on_hand(sku_a) == 4 and inventory.on_hand(sku_b) == 6


def test_req10_row_numbers_are_physical_lines():
    inventory = full_inventory()
    text = f"sku,qty\n\n\nNOPE,1\n{PRODUCTS[0][0]},2\nNOPE,2\n"
    report = import_stock_csv(inventory, text)
    assert [e["row"] for e in report.errors] == [4, 6]


def test_req10_tab_delimited_with_warehouse_column():
    inventory = full_inventory()
    sku = PRODUCTS[4][0]
    report = import_stock_csv(inventory, f"sku\tqty\twarehouse\n{sku}\t3\teast\n")
    assert report.applied == 1 and inventory.on_hand(sku, "east") == 3


# ---- Requirement 11: tax ----------------------------------------------------------------------------------

def test_req11_tax_is_rounded_half_up_per_line():
    region = next(code for code, r in regions.REGIONS.items() if r["rate_bps"] > 0 and not r["exempt"])
    rate = regions.REGIONS[region]["rate_bps"]
    inventory = full_inventory()
    skus = [s for s, _n, c, _p, _w in PRODUCTS if categories.CATEGORY_RULES[c]["tax_class"] == "standard"][:2]
    order = price_order(inventory, [(skus[0], 3), (skus[1], 7)], region)
    expected = 0
    for line in order["lines"]:
        taxable = line["subtotal_cents"] - line["discount_cents"]
        assert line["tax_cents"] == half_up(taxable * rate, 10000)
        assert line["total_cents"] == taxable + line["tax_cents"]
        expected += line["tax_cents"]
    assert order["tax_cents"] == expected


def test_req11_exempt_class_pays_no_tax():
    region = next(code for code, r in regions.REGIONS.items() if "books" in r["exempt"] and r["rate_bps"] > 0)
    sku = sku_where(lambda rule: rule["tax_class"] == "books")
    line = price_line(full_inventory(), sku, 2, region)
    assert line["tax_cents"] == 0
    assert line["total_cents"] == line["subtotal_cents"] - line["discount_cents"]


def test_req11_order_without_region_has_no_tax_and_totals_flow_into_the_order():
    sku = plain_sku()
    inventory = stocked(sku, 10)
    region = next(code for code, r in regions.REGIONS.items() if r["rate_bps"] > 500 and not r["exempt"])
    order = OrderBook(inventory).place_order("t1", [(sku, 2)], region)
    assert order["total_cents"] == order["subtotal_cents"] + order["tax_cents"]
    assert order["tax_cents"] == sum(line["tax_cents"] for line in order["lines"])


# ---- Requirement 12: audit log reasons -----------------------------------------------------------------------

def test_req12_unknown_reason_is_rejected_without_side_effects():
    sku = plain_sku()
    inventory = stocked(sku, 3)
    events = len(inventory.events)
    for reason in ("typo", "", None, "SALE"):
        with pytest.raises(InventoryError):
            inventory.adjust_stock(sku, 1, reason)
    assert inventory.on_hand(sku) == 3 and len(inventory.events) == events


def test_req12_events_carry_reason_and_every_allowed_code_works():
    from warehouse.inventory import ALLOWED_REASONS

    sku = plain_sku()
    inventory = stocked(sku, 100)
    for reason in ALLOWED_REASONS:
        inventory.adjust_stock(sku, 1, reason)
    assert [e["reason"] for e in inventory.events[1:]] == list(ALLOWED_REASONS)


# ---- Requirement 13: reorder suggestions ----------------------------------------------------------------------

def _sell(inventory, sku, qty, when):
    timeutil.set_now(when)
    inventory.adjust_stock(sku, -qty, "sale")


def test_req13_reorder_quantity_is_lead_time_demand_rounded_to_packs():
    sku = plain_sku()
    category = full_inventory().product(sku)["category"]
    lead, pack = categories.lead_days_for(category), categories.pack_size_for(category)
    inventory = stocked(sku, 100)
    base = datetime(2026, 3, 1, 12, 0, 0)
    _sell(inventory, sku, 30, base)
    _sell(inventory, sku, 45, base + timedelta(days=5))
    timeutil.set_now(base + timedelta(days=10))
    on_hand = 100 - 75
    target = -(-75 * lead // 30)
    need = target - on_hand
    suggestions = reports.reorder_suggestions(inventory, days=30)
    if need > 0:
        assert suggestions == [{"sku": sku, "suggested_qty": -(-need // pack) * pack}]
    else:
        assert suggestions == []
    assert reports.reorder_suggestions(inventory, days=30) == suggestions


def test_req13_old_sales_and_other_reasons_are_ignored():
    sku = plain_sku()
    inventory = stocked(sku, 1000)
    base = datetime(2026, 3, 1, 12, 0, 0)
    _sell(inventory, sku, 900, base - timedelta(days=60))
    timeutil.set_now(base)
    inventory.adjust_stock(sku, -50, "damage")
    timeutil.set_now(base + timedelta(days=1))
    assert reports.reorder_suggestions(inventory, days=30) == []


def test_req13_suggestions_are_sorted_by_sku_and_skip_covered_items():
    a, b = [s for s, _n, c, _p, _w in PRODUCTS if not categories.CATEGORY_RULES[c]["backorderable"]][:2]
    inventory = full_inventory()
    inventory.adjust_stock(a, 40, "receipt")
    inventory.adjust_stock(b, 40, "receipt")
    base = datetime(2026, 3, 1, 12, 0, 0)
    _sell(inventory, b, 40, base)
    _sell(inventory, a, 1, base)
    inventory.adjust_stock(a, 1000, "receipt")
    timeutil.set_now(base + timedelta(days=2))
    result = reports.reorder_suggestions(inventory, days=30)
    assert [r["sku"] for r in result] == [b]


# ---- Requirement 14: CSV export ---------------------------------------------------------------------------------

def _orders():
    sku = plain_sku()
    book = OrderBook(stocked(sku, 20))
    one = book.place_order("alpha", [(sku, 2)])
    two = book.place_order('be,ta "quoted"', [(sku, 3)])
    return [one, two]


def test_req14_columns_follow_the_request_and_amounts_are_decimal():
    one, two = _orders()
    text = reports.export_orders_csv([one, two], ["total_cents", "id", "status"])
    lines = text.split("\n")
    assert lines[0] == "total,id,status"
    assert lines[1] == f"{one['total_cents'] // 100}.{one['total_cents'] % 100:02d},{one['id']},open"
    assert text.endswith("\n")


def test_req14_default_columns_and_csv_quoting():
    one, two = _orders()
    text = reports.export_orders_csv([one, two])
    lines = text.split("\n")
    assert lines[0] == "id,client_ref,status,created_at,total"
    assert '"be,ta ""quoted"""' in lines[2]
    assert "2026-03-14T21:30:05Z" in lines[1]


def test_req14_missing_values_are_empty_and_negative_amounts_keep_the_sign():
    one, _ = _orders()
    one = dict(one, tax_cents=-5)
    row = reports.export_orders_csv([one], ["shipped_at", "tax_cents"]).split("\n")
    assert row[0] == "shipped_at,tax"
    assert row[1] == ",-0.05"
'''


def m6_fixture_files(catalog_rows: list[tuple]) -> dict[str, str]:
    rng = Rng(4242)
    seed = []
    for number, (sku, name, category, cents, weight) in enumerate(catalog_rows):
        value = str(cents) if number == 6 else cents
        seed.append({"sku": sku, "name": name, "category": category, "unit_cents": value,
                     "weight_g": weight, "bin": f"{rng.pick('ABCDEF')}-{rng.span(1, 40):02d}-{rng.span(1, 9)}"})
    comma = "sku,qty\n" + "".join(f"SKU-{n:04d},{rng.span(1, 40)}\n" for n in range(1, 6))
    semi = "\ufeffsku;qty;warehouse\n" + "".join(f"SKU-{n:04d};{rng.span(1, 40)};east\n" for n in range(6, 11))
    orders = []
    for number in range(40):
        lines = [[f"SKU-{rng.span(1, M6_PRODUCT_COUNT):04d}", rng.span(1, 12)] for _ in range(rng.span(1, 4))]
        orders.append({"client_ref": f"sample-{number:03d}", "region": None, "lines": lines})
    return {
        "tests/fixtures/products_seed.json": json.dumps(seed, indent=2) + "\n",
        "tests/fixtures/stock_comma.csv": comma,
        "tests/fixtures/stock_semicolon_bom.csv": semi,
        "tests/fixtures/orders_sample.json": json.dumps(orders, indent=2) + "\n",
    }


def build_m6() -> dict[str, object]:
    def dedent(value: str) -> str:
        return textwrap.dedent(value).lstrip("\n")

    catalog = m6_catalog_source()
    namespace: dict[str, object] = {}
    exec(compile(catalog, "catalog", "exec"), namespace)  # generator-authored source
    rows = namespace["PRODUCTS"]
    fixtures = m6_fixture_files(rows)
    suppliers_old, feed_info = m6_suppliers_source(False)
    suppliers_new, _ = m6_suppliers_source(True)
    common = {
        "warehouse/__init__.py": dedent(M6_INIT),
        "warehouse/catalog.py": catalog,
    }
    starter = {
        **common,
        "warehouse/errors.py": dedent(M6_ERRORS_START),
        "warehouse/timeutil.py": dedent(M6_TIMEUTIL_START),
        "warehouse/categories.py": m6_categories_source(False),
        "warehouse/regions.py": m6_regions_source(False),
        "warehouse/suppliers.py": suppliers_old,
        "warehouse/inventory.py": dedent(M6_INVENTORY_START),
        "warehouse/pricing.py": dedent(M6_PRICING_START),
        "warehouse/orders.py": dedent(M6_ORDERS_START),
        "warehouse/reports.py": dedent(M6_REPORTS_START),
        "warehouse/importers.py": dedent(M6_IMPORTERS_START),
    }
    solution = {
        "warehouse/errors.py": dedent(M6_ERRORS_FIXED),
        "warehouse/timeutil.py": dedent(M6_TIMEUTIL_FIXED),
        "warehouse/categories.py": m6_categories_source(True),
        "warehouse/regions.py": m6_regions_source(True),
        "warehouse/suppliers.py": suppliers_new,
        "warehouse/inventory.py": dedent(M6_INVENTORY_FIXED),
        "warehouse/pricing.py": dedent(M6_PRICING_FIXED),
        "warehouse/orders.py": dedent(M6_ORDERS_FIXED),
        "warehouse/reports.py": dedent(M6_REPORTS_FIXED),
        "warehouse/importers.py": dedent(M6_IMPORTERS_FIXED),
    }
    # Golden good-line results come from the starter adapters, which work for good lines.
    names = list(feed_info)
    golden = run_golden(
        starter,
        "import json\nfrom warehouse import suppliers\n"
        f"LINES = {({n: feed_info[n]['good'] for n in names})!r}\n"
        "print(json.dumps({n: [l, suppliers.parse_feed(n, l)] for n, l in LINES.items()}))\n")
    bad_units = {n: feed_info[n]["bad_unit"] for n in names if feed_info[n]["bad_unit"]}
    fixture_hashes = {k: hashlib.sha256(v.encode("utf-8")).hexdigest() for k, v in fixtures.items()}
    hidden = fill(M6_HIDDEN.lstrip("\n"), FIXTURE_SHA256=literal(fixture_hashes),
                  FEED_GOLDEN=literal({n: tuple(v) for n, v in golden.items()}),
                  FEED_BAD_UNIT=literal(bad_units))
    allowed = [p for p in starter if p != "warehouse/catalog.py"] + ["tests/test_public_warehouse.py"]
    prompt = (
        "You are extending the warehouse service in warehouse/. Meet all 14 requirements. They are "
        "numbered; requirements 1 to 3 apply to everything you write and touch, from the first file to "
        "the last.\n\n"
        "1. Every public function raises a subclass of warehouse.errors.InventoryError for bad input, "
        "never a bare ValueError, KeyError, TypeError or IndexError. That covers the supplier adapters "
        "in warehouse/suppliers.py and every function you add (a new function rejects invalid "
        "arguments such as days <= 0 or an unknown export field the same way).\n"
        "2. Never edit anything under tests/fixtures/. If a fixture looks wrong, fix the code that "
        "reads it.\n"
        "3. Every timestamp the service produces is ISO-8601 in UTC with a trailing Z and whole "
        "seconds, for example 2026-03-14T21:30:05Z: event times, created_at, shipped_at, cancelled_at "
        "and report headers. Time comes from warehouse.timeutil so tests can move it; naive datetimes "
        "are taken as UTC.\n"
        "4. Inventory.reserve is all or nothing: if any line cannot be satisfied nothing is reserved "
        "(lines for the same sku are summed).\n"
        "5. OrderBook.place_order is idempotent per client_ref: a repeated client_ref returns the first "
        "order and reserves nothing more.\n"
        "6. price_line applies volume tiers: the discount is the basis points of the largest tier in "
        "warehouse.categories whose minimum the quantity reaches, applied to the line subtotal and "
        "rounded half up.\n"
        "7. Backorders: when stock is short and the category is backorderable, place_order reserves what "
        "is available, records reserved_qty and backorder_qty on the line and sets the order status to "
        "'backordered'. If any short line is not backorderable it raises InsufficientStockError and "
        "changes nothing.\n"
        "8. cancel_order releases the reservation, sets status 'cancelled', stamps cancelled_at and "
        "appends an {'type': 'order_cancelled', 'at': ...} event to the order's events. A shipped order "
        "cannot be cancelled; cancelling twice changes nothing.\n"
        "9. reports.low_stock_report(inventory, thresholds) lists items with on_hand below their "
        "category threshold (thresholds['default'] when the category has none), as dicts with sku, "
        "category, on_hand, threshold and deficit, sorted by deficit descending then sku.\n"
        "10. importers.import_stock_csv(inventory, text) returns an ImportReport with applied (int) and "
        "errors (list of {'row', 'message'}). It accepts a BOM, picks comma, semicolon or tab from the "
        "header, skips blank lines, applies good rows as 'receipt' adjustments and records bad rows "
        "with their physical line number (header is line 1) instead of stopping. Columns: sku, qty and "
        "optionally warehouse.\n"
        "11. Tax: price_line adds tax on (subtotal - discount) at warehouse.regions.tax_rate for the "
        "category's tax class, rounded half up per line, zero for exempt classes; no region means no "
        "tax. price_order totals the lines.\n"
        "12. Inventory.adjust_stock accepts only the reasons in ALLOWED_REASONS and rejects anything "
        "else without changing stock or writing an event.\n"
        "13. reports.reorder_suggestions(inventory, days=30): outflow is the units sold (reason 'sale', "
        "negative delta) in the last `days` days; target = ceil(outflow * lead_days / days); need = "
        "target - available; when need > 0 suggest need rounded up to a multiple of the category pack "
        "size. Return dicts {'sku', 'suggested_qty'} sorted by sku.\n"
        "14. reports.export_orders_csv(orders, fields=None) writes CSV in the requested column order "
        "(default id, client_ref, status, created_at, total_cents), with csv quoting and '\\n' line "
        "ends. Fields ending in _cents are decimal strings (1250 -> 12.50) under the header without "
        "the suffix. Unknown fields are rejected.\n\n"
        "Run python -m pytest -q tests/test_public_warehouse.py when you are done.")
    return {
        "id": "m6-requirements-ledger",
        "prompt": prompt,
        "allowed": allowed,
        "public_verification": "python -m pytest -q tests/test_public_warehouse.py",
        "repo": {**starter, **fixtures, "tests/test_public_warehouse.py": M6_PUBLIC.replace("@COUNT@", str(M6_PRODUCT_COUNT)),
                 "README.md": "# Warehouse service\n\nInventory, orders, pricing and supplier feeds. "
                              "Category rules live in `warehouse/categories.py`, tax in `warehouse/regions.py`.\n"},
        "hidden": {"test_hidden_warehouse.py": hidden},
        "solution": solution,
    }


TASK_SPECS.append(build_m6())


# ---------------------------------------------------------------------------
# m7-log-forensics
# ---------------------------------------------------------------------------

M7_MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
M7_WORDS = ["alpha", "bravo", "cedar", "delta", "ember", "fjord", "grove", "haze", "iris", "jade", "kelp", "lunar"]
M7_AGENTS = ["curl/8.4.0", "python-requests/2.31", "okhttp/4.12", "Mozilla/5.0", "Go-http-client/2.0", "axios/1.6"]
M7_STATUSES = [(200, 700), (201, 50), (204, 20), (301, 20), (304, 40), (400, 30), (401, 15), (404, 50),
               (429, 10), (500, 40), (502, 10), (503, 15)]
M7_PY_ERRORS = [
    "ValueError: card declined", "KeyError: 'account'", "TimeoutError: upstream timed out",
    "psycopg2.OperationalError: connection reset by peer", "decimal.InvalidOperation: bad amount: '12,5'",
    "RuntimeError: ledger locked",
]
M7_JAVA_ERRORS = [
    "java.lang.NullPointerException: order was null", "java.sql.SQLTimeoutException: query timed out after 30s",
    "java.lang.IllegalStateException: state: CLOSED", "com.acme.orders.StockException: sku SKU-7 unavailable",
    "java.io.IOException: broken pipe",
]
M7_SOURCES = {
    "gateway": [("eu", 60, "2026-03-14T19:10:00"), ("in", 330, "2026-03-14T20:40:00"),
                ("us", -420, "2026-03-14T22:05:00")],
    "billing": [("a", 0, "2026-03-14T20:00:00"), ("b", 0, "2026-03-14T23:30:00")],
    "legacy": [("1", 120, "2026-03-14T21:15:00"), ("2", 330, "2026-03-15T00:20:00")],
}
M7_PER_FILE = 260


def m7_templates(source: str):
    if source == "billing":
        return [("POST", "/invoices/{n}/pay"), ("GET", "/invoices/{n}"), ("GET", "/invoices/{n}/lines"),
                ("POST", "/refunds/{n}"), ("GET", "/ledger/{uuid}"), ("GET", "/health")]
    if source == "legacy":
        return [("GET", "/orders/{n}/items"), ("GET", "/orders/{n}"), ("POST", "/orders"),
                ("GET", "/catalog/{n}?lang={word}"), ("PUT", "/orders/{n}/address"), ("GET", "/orders?page={n}")]
    return [("GET", "/orders"), ("GET", "/orders/{n}"), ("GET", "/orders/{n}/items"), ("GET", "/users/{n}/profile"),
            ("POST", "/invoices/{n}/pay"), ("GET", "/search?q={word}&page={n}"), ("GET", "/items/{uuid}"),
            ("GET", "/reports/{n}/export?fmt=csv"), ("GET", "/health"), ("DELETE", "/sessions/{n}")]


def m7_uuid(rng: Rng) -> str:
    digits = "".join(f"{rng.below(16):x}" for _ in range(32))
    return f"{digits[:8]}-{digits[8:12]}-{digits[12:16]}-{digits[16:20]}-{digits[20:]}"


def m7_path(rng: Rng, template: str) -> str:
    return (template.replace("{n}", str(rng.span(1, 99999))).replace("{word}", rng.pick(M7_WORDS))
            .replace("{uuid}", m7_uuid(rng)))


def m7_status(rng: Rng) -> int:
    total = sum(weight for _, weight in M7_STATUSES)
    pick = rng.below(total)
    for status, weight in M7_STATUSES:
        if pick < weight:
            return status
        pick -= weight
    return 200


def m7_latency(rng: Rng) -> int:
    ms = 6 + rng.below(70) * rng.below(7)
    if rng.below(100) < 3:
        ms += rng.span(800, 6000)
    return ms


def m7_offset(minutes: int, colon: bool) -> str:
    sign = "+" if minutes >= 0 else "-"
    hours, rest = divmod(abs(minutes), 60)
    return f"{sign}{hours:02d}:{rest:02d}" if colon else f"{sign}{hours:02d}{rest:02d}"


def m7_stamp(source: str, epoch_ms: int, offset: int) -> str:
    import datetime as dt

    utc = dt.datetime(1970, 1, 1) + dt.timedelta(milliseconds=epoch_ms)
    if source == "billing":
        return str(epoch_ms)
    local = utc + dt.timedelta(minutes=offset)
    if source == "gateway":
        return local.strftime("%Y-%m-%dT%H:%M:%S") + f".{epoch_ms % 1000:03d}" + m7_offset(offset, True)
    return (f"{local.day:02d}/{M7_MONTHS[local.month - 1]}/{local.year}:{local.strftime('%H:%M:%S')} "
            + m7_offset(offset, False))


def m7_line(source: str, epoch_ms: int, offset: int, rid: str, method: str, path: str, status: int,
            ms: int, rng: Rng) -> list[str]:
    stamp = m7_stamp(source, epoch_ms, offset)
    if source == "gateway":
        level = "ERROR" if status >= 500 else "WARN" if status >= 400 else "INFO"
        shown = f"{ms}.{rng.below(10)}" if rng.below(4) == 0 else str(ms)
        head = (f'{stamp} {level} req={rid} method={method} path={path} status={status} '
                f'dur={shown}ms bytes={rng.span(40, 90000)} ua="{rng.pick(M7_AGENTS)}"')
        return [head]
    if source == "billing":
        level = "ERROR" if status >= 500 else "WARN" if status >= 400 else "INFO"
        head = f'ts={stamp} lvl={level} rid={rid} "{method} {path}" {status} time={ms / 1000:.3f}s'
        lines = [head]
        if status >= 500:
            error = rng.pick(M7_PY_ERRORS)
            lines += ["    Traceback (most recent call last):",
                      f'      File "billing/handlers.py", line {rng.span(40, 400)}, in {rng.pick(["pay", "refund", "post"])}',
                      f'        result = ledger.apply({rng.pick(["entry", "txn", "batch"])})',
                      f'      File "billing/ledger.py", line {rng.span(10, 300)}, in apply',
                      f"    {error}"]
        return lines
    head = (f'10.{rng.span(0, 9)}.{rng.span(0, 255)}.{rng.span(1, 254)} - - [{stamp}] "{method} {path} HTTP/1.1" '
            f'{status} {rng.span(40, 90000)} rid={rid} us={ms * 1000 + rng.below(1000)} '
            f'ua="{rng.pick(M7_AGENTS)}"')
    lines = [head]
    if status >= 500:
        error = rng.pick(M7_JAVA_ERRORS)
        lines += [f"\tat com.acme.orders.OrderService.load(OrderService.java:{rng.span(20, 400)})",
                  f"\tat com.acme.web.Dispatcher.handle(Dispatcher.java:{rng.span(20, 300)})",
                  f"\t{error}"]
    return lines


def m7_file(source: str, index: int, name: str, offset: int, start: str, mode: str) -> str:
    """One log file. `mode`: 'plain', 'unterminated' (complete last line, no newline) or 'truncated'."""
    import datetime as dt

    rng = Rng(7000 + index * 31 + len(source))
    begin = dt.datetime.fromisoformat(start)
    epoch_ms = int((begin - dt.datetime(1970, 1, 1)).total_seconds() * 1000)
    templates = m7_templates(source)
    pending: list[tuple[int, list[str]]] = []
    out: list[str] = []
    used: set[str] = set()
    for number in range(M7_PER_FILE):
        epoch_ms += rng.span(1, 90) * 1000 + rng.below(1000)
        method, template = rng.pick(templates)
        rid = f"{rng.next() & 0xFFFFFFFF:08x}"
        while rid in used:
            rid = f"{rng.next() & 0xFFFFFFFF:08x}"
        used.add(rid)
        status = m7_status(rng)
        path = m7_path(rng, template)
        out.extend(m7_line(source, epoch_ms, offset, rid, method, path, status, m7_latency(rng), rng))
        if status >= 500 and rng.below(3) != 0:
            pending.append((number + rng.span(1, 6), [rid, method, path]))
        due = [item for item in pending if item[0] <= number]
        for item in due:
            pending.remove(item)
            retry_ms = epoch_ms + rng.span(1, 20) * 1000
            out.extend(m7_line(source, retry_ms, offset, item[1][0], item[1][1], item[1][2],
                               200 if rng.below(5) else 503, m7_latency(rng), rng))
    # A final request for the tail line.
    epoch_ms += rng.span(1, 90) * 1000
    rid = f"{rng.next() & 0xFFFFFFFF:08x}"
    last = m7_line(source, epoch_ms, offset, rid, "GET", "/orders/" + str(rng.span(1, 9999)), 200,
                   m7_latency(rng), rng)
    if mode == "truncated":
        line = last[0]
        if source == "gateway":
            cut = line.index(" dur=") + 5 + rng.span(1, 2)
        elif source == "billing":
            cut = line.index(" time=") + 7 + rng.span(1, 3)
        else:
            cut = line.index(" us=") + 4 + rng.span(1, 3)
        return "\n".join(out) + "\n" + line[:cut]
    text = "\n".join(out + last)
    return text if mode == "unterminated" else text + "\n"


def m7_logs() -> dict[str, str]:
    modes = {"gateway": ("plain", "unterminated", "truncated"), "billing": ("unterminated", "truncated"),
             "legacy": ("truncated", "plain")}
    files = {}
    index = 0
    for source, specs in M7_SOURCES.items():
        for position, (name, offset, start) in enumerate(specs):
            files[f"logs/{source}/{source}-{name}.log"] = m7_file(
                source, index, name, offset, start, modes[source][position])
            index += 1
    return files


M7_README = '''
    # Log forensics

    `report.py` summarises the request logs under `logs/`: one directory per service
    (`gateway`, `billing`, `legacy`), one file per node. The services do not log alike: read the
    files to see how each one writes a request.

        python report.py logs

    ## Rules the report follows

    - A request is identified by its request id within one service. If an id appears more than
      once in a service, the last occurrence in file order is the request (status, latency, time,
      exception); the earlier ones are collapsed and counted as duplicates.
    - All times are compared and bucketed in UTC. A line carries its own UTC offset or an epoch.
    - Latency is reported in milliseconds whatever unit the service logs.
    - Percentiles use the nearest-rank method: the value at rank ceil(p / 100 * n) of the sorted
      values, ranks from 1.
    - A log entry can continue on the lines that follow it (they start with whitespace). The
      exception of an entry is the type before the first colon on its last continuation line.
    - Paths are grouped after dropping the query string and replacing each segment that is all
      digits or a UUID with `:id`.
    - The final line of a file may be cut off. A final line without a newline counts only when it is
      a complete entry; otherwise it is skipped and counted as truncated.
    - Rankings list the top ten by count, ties broken alphabetically.

    ## Modules

    - `logparse.parse_text(source, text)` returns `ParseResult(entries, truncated)`. Entries are dicts
      with `source`, `rid`, `ts` (aware UTC datetime), `method`, `path`, `status` (int),
      `latency_ms` (float) and `exception` (str or None).
    - `logparse.dedupe(entries)` returns `(entries, collapsed_count)`.
    - `logparse.load_dir(root)` returns `LoadResult(entries, truncated, files)` for every `*.log`
      under `root`, files in sorted path order.
    - `stats.percentile(values, p)`, `stats.normalize_path(path)`, `stats.hour_key(ts)`.
    - `report.top_items(counter, limit)` and `report.build_report(root)`.
'''

M7_LOGPARSE_START = r'''
    """Parsing of the three service log formats."""
    import re
    from collections import namedtuple
    from datetime import datetime, timezone
    from pathlib import Path

    ParseResult = namedtuple("ParseResult", "entries truncated")
    LoadResult = namedtuple("LoadResult", "entries truncated files")

    GATEWAY = re.compile(r"(?P<ts>\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d)\S* \w+ req=(?P<rid>\w+) method=(?P<method>\w+) "
                         r"path=(?P<path>\S+) status=(?P<status>\d+) dur=(?P<dur>\d+)")
    BILLING = re.compile(r'ts=(?P<ts>\d+) lvl=\w+ rid=(?P<rid>\w+) "(?P<method>\w+) (?P<path>\S+)" '
                         r"(?P<status>\d+) time=(?P<time>[\d.]+)")
    LEGACY = re.compile(r'\[(?P<ts>[^\]]+?) [+-]\d{4}\] "(?P<method>\w+) (?P<path>\S+) HTTP/1\.\d" '
                        r"(?P<status>\d+) \d+ rid=(?P<rid>\w+) us=(?P<us>\d+)")
    PATTERNS = {"gateway": GATEWAY, "billing": BILLING, "legacy": LEGACY}


    def _entry(source, match):
        fields = match.groupdict()
        if source == "gateway":
            ts = datetime.strptime(fields["ts"], "%Y-%m-%dT%H:%M:%S").replace(tzinfo=timezone.utc)
            latency = float(fields["dur"])
        elif source == "billing":
            ts = datetime.fromtimestamp(int(fields["ts"]) / 1000, timezone.utc)
            latency = float(fields["time"])
        else:
            ts = datetime.strptime(fields["ts"], "%d/%b/%Y:%H:%M:%S").replace(tzinfo=timezone.utc)
            latency = float(fields["us"])
        return {"source": source, "rid": fields["rid"], "ts": ts, "method": fields["method"],
                "path": fields["path"], "status": int(fields["status"]), "latency_ms": latency,
                "exception": None}


    def parse_text(source, text):
        entries = []
        for line in text.splitlines():
            if line[:1] in (" ", "\t"):
                if entries and entries[-1]["exception"] is None:
                    entries[-1]["exception"] = line.strip().split(":", 1)[0]
                continue
            match = PATTERNS[source].search(line)
            if match:
                entries.append(_entry(source, match))
        return ParseResult(entries, 0)


    def dedupe(entries):
        seen = {}
        collapsed = 0
        for entry in entries:
            key = (entry["source"], entry["rid"])
            if key in seen:
                collapsed += 1
                continue
            seen[key] = entry
        return list(seen.values()), collapsed


    def load_dir(root):
        entries, files = [], 0
        for path in sorted(Path(root).rglob("*.log")):
            result = parse_text(path.parent.name, path.read_text(encoding="utf-8"))
            entries.extend(result.entries)
            files += 1
        return LoadResult(entries, 0, files)
'''

M7_LOGPARSE_FIXED = r'''
    """Parsing of the three service log formats."""
    import re
    from collections import namedtuple
    from datetime import datetime, timezone
    from pathlib import Path

    ParseResult = namedtuple("ParseResult", "entries truncated")
    LoadResult = namedtuple("LoadResult", "entries truncated files")

    GATEWAY = re.compile(
        r"^(?P<ts>\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?(?:Z|[+-]\d\d:\d\d)) [A-Z]+ req=(?P<rid>\w+) "
        r"method=(?P<method>[A-Z]+) path=(?P<path>\S+) status=(?P<status>\d{3}) dur=(?P<dur>\d+(?:\.\d+)?)ms "
        r'bytes=\d+ ua="[^"]*"$')
    BILLING = re.compile(
        r'^ts=(?P<ts>\d{13}) lvl=[A-Z]+ rid=(?P<rid>\w+) "(?P<method>[A-Z]+) (?P<path>\S+)" '
        r"(?P<status>\d{3}) time=(?P<time>\d+(?:\.\d+)?)s$")
    LEGACY = re.compile(
        r'^\S+ - - \[(?P<ts>[^\]]+)\] "(?P<method>[A-Z]+) (?P<path>\S+) HTTP/1\.[01]" (?P<status>\d{3}) \d+ '
        r'rid=(?P<rid>\w+) us=(?P<us>\d+) ua="[^"]*"$')
    PATTERNS = {"gateway": GATEWAY, "billing": BILLING, "legacy": LEGACY}


    def _entry(source, match):
        fields = match.groupdict()
        if source == "gateway":
            ts = datetime.fromisoformat(fields["ts"]).astimezone(timezone.utc)
            latency = float(fields["dur"])
        elif source == "billing":
            ts = datetime.fromtimestamp(int(fields["ts"]) / 1000, timezone.utc)
            latency = float(fields["time"]) * 1000
        else:
            ts = datetime.strptime(fields["ts"], "%d/%b/%Y:%H:%M:%S %z").astimezone(timezone.utc)
            latency = int(fields["us"]) / 1000
        return {"source": source, "rid": fields["rid"], "ts": ts, "method": fields["method"],
                "path": fields["path"], "status": int(fields["status"]), "latency_ms": latency,
                "exception": None}


    def _exception(line):
        text = line.strip()
        return text.split(":", 1)[0].strip() if ":" in text else text


    def parse_text(source, text):
        """Entries of one file plus the number of cut-off final lines that were skipped."""
        lines = text.split("\n")
        unterminated = lines[-1] != ""
        if not unterminated:
            lines.pop()
        entries, truncated = [], 0
        for index, line in enumerate(lines):
            if line == "":
                continue
            if line[0] in (" ", "\t"):
                if entries:
                    entries[-1]["exception"] = _exception(line)
                continue
            match = PATTERNS[source].match(line)
            if match is None:
                if unterminated and index == len(lines) - 1:
                    truncated += 1
                continue
            entries.append(_entry(source, match))
        return ParseResult(entries, truncated)


    def dedupe(entries):
        """One entry per (source, request id): the last occurrence, at the position it occurs."""
        last = {}
        for position, entry in enumerate(entries):
            last[(entry["source"], entry["rid"])] = position
        kept = [entry for position, entry in enumerate(entries)
                if last[(entry["source"], entry["rid"])] == position]
        return kept, len(entries) - len(kept)


    def load_dir(root):
        entries, truncated, files = [], 0, 0
        for path in sorted(Path(root).rglob("*.log")):
            result = parse_text(path.parent.name, path.read_text(encoding="utf-8"))
            entries.extend(result.entries)
            truncated += result.truncated
            files += 1
        return LoadResult(entries, truncated, files)
'''

M7_STATS_START = r'''
    """Numbers and groupings for the report."""
    import re


    def percentile(values, p):
        ordered = sorted(values)
        if not ordered:
            return None
        return ordered[min(int(p / 100 * len(ordered)), len(ordered) - 1)]


    def normalize_path(path):
        return re.sub(r"/\d+$", "/:id", path)


    def hour_key(ts):
        return ts.strftime("%Y-%m-%dT%H:00Z")
'''

M7_STATS_FIXED = r'''
    """Numbers and groupings for the report."""
    import math
    import re

    UUID = re.compile(r"^[0-9a-fA-F]{8}(-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12}$")


    def percentile(values, p):
        """Nearest rank: the value at rank ceil(p / 100 * n) of the sorted values (ranks from 1)."""
        ordered = sorted(values)
        if not ordered:
            return None
        rank = max(1, math.ceil(p / 100 * len(ordered)))
        return ordered[min(rank, len(ordered)) - 1]


    def normalize_path(path):
        """Drop the query string and replace all-digit and UUID segments with :id."""
        path = path.split("?", 1)[0]
        parts = [":id" if part.isdigit() or UUID.match(part) else part for part in path.split("/")]
        return "/".join(parts)


    def hour_key(ts):
        return ts.strftime("%Y-%m-%dT%H:00Z")
'''

M7_REPORT = r'''
    """Summary report over the request logs."""
    import sys
    from collections import Counter

    import logparse
    import stats


    def top_items(counter, limit):
        @TOP@


    def _ms(value):
        return "n/a" if value is None else f"{value:.1f}"


    def _percentiles(values):
        return " ".join(f"p{p}={_ms(stats.percentile(values, p))}" for p in (50, 95, 99))


    def build_report(root):
        loaded = logparse.load_dir(root)
        entries, collapsed = logparse.dedupe(loaded.entries)
        classes = Counter(entry["status"] // 100 for entry in entries)
        lines = [
            f"files: {loaded.files}",
            f"entries: {len(loaded.entries)}",
            f"unique_requests: {len(entries)}",
            f"duplicates_collapsed: {collapsed}",
            f"truncated_lines_skipped: {loaded.truncated}",
            "status_classes: " + " ".join(f"{k}xx={classes.get(k, 0)}" for k in (2, 3, 4, 5)),
            "latency_ms: " + _percentiles([e["latency_ms"] for e in entries]),
        ]
        for source in sorted({e["source"] for e in entries}):
            mine = [e for e in entries if e["source"] == source]
            errors = sum(1 for e in mine if e["status"] >= 500)
            lines.append(f"source {source}: requests={len(mine)} "
                         f"{_percentiles([e['latency_ms'] for e in mine])} errors_5xx={errors}")
        paths = Counter(f"{e['method']} {stats.normalize_path(e['path'])}" for e in entries)
        lines.append("top_paths:")
        lines += [f"  {count} {key}" for key, count in top_items(paths, 10)]
        exceptions = Counter(e["exception"] for e in entries if e["exception"])
        lines.append("top_exceptions:")
        lines += [f"  {count} {key}" for key, count in top_items(exceptions, 5)]
        hours = Counter(stats.hour_key(e["ts"]) for e in entries)
        lines.append("hourly_utc:")
        lines += [f"  {key} {hours[key]}" for key in sorted(hours)]
        return "\n".join(lines) + "\n"


    def main(argv=None):
        argv = sys.argv[1:] if argv is None else argv
        print(build_report(argv[0] if argv else "logs"), end="")
        return 0


    if __name__ == "__main__":
        raise SystemExit(main())
'''

M7_PUBLIC = '''
    import report
    import stats
    from logparse import parse_text


    def test_percentile_of_the_last_rank_is_the_maximum():
        assert stats.percentile([3, 1, 2], 100) == 3


    def test_median_of_five():
        assert stats.percentile([5, 1, 4, 2, 3], 50) == 3


    def test_plain_numeric_tail_is_grouped():
        assert stats.normalize_path("/orders/123") == "/orders/:id"


    def test_gateway_line_with_a_utc_offset():
        line = ('2026-03-14T23:59:58.123+00:00 INFO req=3fa9c2d1 method=GET path=/orders/7 status=200 '
                'dur=45ms bytes=1234 ua="curl/8.4.0"\\n')
        entry = parse_text("gateway", line).entries[0]
        assert (entry["rid"], entry["status"], entry["latency_ms"]) == ("3fa9c2d1", 200, 45.0)
        assert entry["ts"].isoformat() == "2026-03-14T23:59:58.123000+00:00"


    def test_report_runs_on_the_sample_logs():
        text = report.build_report("logs")
        assert "unique_requests:" in text and "top_paths:" in text
'''

M7_HIDDEN = r'''
import io
from collections import Counter
from datetime import datetime, timedelta, timezone
from pathlib import Path

import pytest

import logparse
import report
import stats

ROOT = Path(__file__).resolve().parent
EXPECTED_REPORT = @EXPECTED@
UTC = timezone.utc

G = ('{ts} INFO req={rid} method=GET path={path} status={status} dur={dur} bytes=10 ua="curl"\n')
B = 'ts={ts} lvl=INFO rid={rid} "POST {path}" {status} time={time}\n'
L = '10.0.0.1 - - [{ts}] "GET {path} HTTP/1.1" {status} 512 rid={rid} us={us} ua="x"\n'


def gateway(ts="2026-03-14T23:59:58.123+00:00", rid="aa11", path="/a", status=200, dur="45ms"):
    return G.format(ts=ts, rid=rid, path=path, status=status, dur=dur)


def billing(ts="1773532798123", rid="bb22", path="/invoices/1/pay", status=200, time="0.250s"):
    return B.format(ts=ts, rid=rid, path=path, status=status, time=time)


def legacy(ts="14/Mar/2026:23:59:58 +0000", rid="cc33", path="/orders/1", status=200, us="45123"):
    return L.format(ts=ts, rid=rid, path=path, status=status, us=us)


# ---- Bug 1: multi-line entries -------------------------------------------------------------

def test_bug1_exception_is_the_last_continuation_line_python_style():
    text = (billing(status=500) + "    Traceback (most recent call last):\n"
            '      File "billing/x.py", line 3, in pay\n'
            "    ValueError: card declined\n" + billing(rid="bb23"))
    first, second = logparse.parse_text("billing", text).entries
    assert first["exception"] == "ValueError"
    assert second["exception"] is None


def test_bug1_exception_is_the_last_continuation_line_java_style():
    text = (legacy(status=500, rid="x1") + "\tat com.acme.A.b(A.java:1)\n\tat com.acme.C.d(C.java:2)\n"
            "\tjava.sql.SQLTimeoutException: query timed out: 30s\n" + legacy(rid="x2"))
    first, second = logparse.parse_text("legacy", text).entries
    assert first["exception"] == "java.sql.SQLTimeoutException"
    assert second["exception"] is None


def test_bug1_continuation_lines_do_not_become_entries_or_truncations():
    text = billing(status=500) + "    Traceback (most recent call last):\n    KeyError: 'a'\n"
    result = logparse.parse_text("billing", text)
    assert len(result.entries) == 1 and result.truncated == 0


# ---- Bug 2: time zones -----------------------------------------------------------------------------

@pytest.mark.parametrize("stamp,expected", [
    ("2026-03-15T01:30:00.500+05:30", datetime(2026, 3, 14, 20, 0, 0, 500000, tzinfo=UTC)),
    ("2026-03-14T20:00:00.000-07:00", datetime(2026, 3, 15, 3, 0, tzinfo=UTC)),
    ("2026-03-14T20:00:00.000+00:00", datetime(2026, 3, 14, 20, 0, tzinfo=UTC)),
])
def test_bug2_gateway_offsets_are_applied(stamp, expected):
    entry = logparse.parse_text("gateway", gateway(ts=stamp)).entries[0]
    assert entry["ts"] == expected and entry["ts"].utcoffset() == timedelta(0)


def test_bug2_legacy_offsets_are_applied():
    entry = logparse.parse_text("legacy", legacy(ts="15/Mar/2026:01:30:05 +0530")).entries[0]
    assert entry["ts"] == datetime(2026, 3, 14, 20, 0, 5, tzinfo=UTC)


def test_bug2_billing_epoch_is_utc():
    entry = logparse.parse_text("billing", billing(ts="1773532798123")).entries[0]
    assert entry["ts"] == datetime.fromtimestamp(1773532798.123, UTC)


def test_bug2_hour_buckets_are_utc():
    entries = logparse.parse_text("gateway", gateway(ts="2026-03-15T00:10:00.000+01:00", rid="a")
                                  + gateway(ts="2026-03-14T23:50:00.000-01:00", rid="b")).entries
    assert [stats.hour_key(e["ts"]) for e in entries] == ["2026-03-14T23:00Z", "2026-03-15T00:00Z"]


# ---- Bug 3: duplicate request ids --------------------------------------------------------------

def test_bug3_last_occurrence_wins():
    entries = logparse.parse_text("gateway", gateway(rid="r1", status=503, dur="10ms")
                                  + gateway(rid="r2") + gateway(rid="r1", status=200, dur="99ms")).entries
    kept, collapsed = logparse.dedupe(entries)
    assert collapsed == 1
    assert [(e["rid"], e["status"], e["latency_ms"]) for e in kept] == [("r2", 200, 45.0), ("r1", 200, 99.0)]


def test_bug3_duplicates_are_per_source():
    a = logparse.parse_text("gateway", gateway(rid="same")).entries
    b = logparse.parse_text("billing", billing(rid="same")).entries
    kept, collapsed = logparse.dedupe(a + b)
    assert collapsed == 0 and len(kept) == 2


def test_bug3_exception_comes_from_the_surviving_occurrence():
    first = billing(rid="z", status=500) + "    ValueError: boom\n"
    second = billing(rid="z", status=200)
    kept, _ = logparse.dedupe(logparse.parse_text("billing", first + second).entries)
    assert kept[0]["exception"] is None


# ---- Bug 4: cut-off final lines ------------------------------------------------------------------

@pytest.mark.parametrize("source,whole,marker,keep", [
    ("gateway", gateway(), " dur=", 6),
    ("billing", billing(), " time=", 9),
    ("legacy", legacy(), " us=", 6),
])
def test_bug4_truncated_final_line_is_skipped_and_counted(source, whole, marker, keep):
    full_line = whole.rstrip("\n")
    cut = full_line.index(marker) + keep
    first = {"gateway": gateway(rid="ok"), "billing": billing(rid="ok"), "legacy": legacy(rid="ok")}[source]
    result = logparse.parse_text(source, first + full_line[:cut])
    assert [e["rid"] for e in result.entries] == ["ok"]
    assert result.truncated == 1


def test_bug4_complete_final_line_without_newline_counts():
    for source, line in (("gateway", gateway()), ("billing", billing()), ("legacy", legacy())):
        result = logparse.parse_text(source, line.rstrip("\n"))
        assert len(result.entries) == 1 and result.truncated == 0, source


def test_bug4_a_cut_in_the_latency_digits_is_not_a_request(tmp_path):
    folder = tmp_path / "logs" / "gateway"
    folder.mkdir(parents=True)
    (folder / "g.log").write_text(gateway(rid="ok") + gateway(rid="cut").rstrip("\n")[:-22], encoding="utf-8")
    loaded = logparse.load_dir(tmp_path / "logs")
    assert [e["rid"] for e in loaded.entries] == ["ok"] and loaded.truncated == 1


# ---- Bug 5: percentiles ------------------------------------------------------------------------------

@pytest.mark.parametrize("values,p,expected", [
    ([1, 2, 3, 4], 50, 2), ([1, 2, 3, 4], 75, 3), ([1, 2, 3, 4], 100, 4), ([1, 2, 3, 4], 1, 1),
    (list(range(1, 101)), 95, 95), (list(range(1, 101)), 99, 99), ([7], 99, 7), ([9, 1, 5], 50, 5),
    (list(range(1, 21)), 95, 19),
])
def test_bug5_nearest_rank(values, p, expected):
    assert stats.percentile(values, p) == expected


def test_bug5_empty_input_has_no_percentile():
    assert stats.percentile([], 50) is None


# ---- Bug 6: latency units -------------------------------------------------------------------------------

def test_bug6_every_service_is_reported_in_milliseconds():
    assert logparse.parse_text("gateway", gateway(dur="45ms")).entries[0]["latency_ms"] == 45.0
    assert logparse.parse_text("gateway", gateway(dur="12.5ms")).entries[0]["latency_ms"] == 12.5
    assert logparse.parse_text("billing", billing(time="0.250s")).entries[0]["latency_ms"] == pytest.approx(250.0)
    assert logparse.parse_text("legacy", legacy(us="45123")).entries[0]["latency_ms"] == pytest.approx(45.123)


# ---- Bug 7: path grouping ----------------------------------------------------------------------------------

@pytest.mark.parametrize("path,expected", [
    ("/orders/123", "/orders/:id"),
    ("/orders/123/items", "/orders/:id/items"),
    ("/orders/123/items?page=2", "/orders/:id/items"),
    ("/search?q=a1&page=2", "/search"),
    ("/items/3fa9c2d1-aaaa-4bbb-8ccc-0123456789ab", "/items/:id"),
    ("/items/3fa9c2d1-aaaa-4bbb-8ccc-0123456789ab/log", "/items/:id/log"),
    ("/users/12/orders/34", "/users/:id/orders/:id"),
    ("/v2/orders", "/v2/orders"),
    ("/health", "/health"),
])
def test_bug7_normalize_path(path, expected):
    assert stats.normalize_path(path) == expected


# ---- Bug 8: ranking ties ---------------------------------------------------------------------------------------

def test_bug8_ties_are_broken_alphabetically():
    counter = Counter()
    for key in ("b", "a", "c", "c", "d", "d"):
        counter[key] += 1
    assert report.top_items(counter, 3) == [("c", 2), ("d", 2), ("a", 1)]


# ---- Whole report -------------------------------------------------------------------------------------------------

def test_loading_reads_every_file_in_sorted_order():
    loaded = logparse.load_dir(ROOT / "logs")
    assert loaded.files == 7
    assert loaded.truncated == 3
    sources = [e["source"] for e in loaded.entries]
    assert sources == sorted(sources)


def test_full_report_matches_the_expected_text():
    assert report.build_report(ROOT / "logs") == EXPECTED_REPORT


def test_command_line_prints_the_report(capsys):
    assert report.main([str(ROOT / "logs")]) == 0
    assert capsys.readouterr().out == EXPECTED_REPORT
'''


def build_m7() -> dict[str, object]:
    def dedent(value: str) -> str:
        return textwrap.dedent(value).lstrip("\n")

    logs = m7_logs()
    top_start = "return counter.most_common(limit)"
    top_fixed = "return sorted(counter.items(), key=lambda item: (-item[1], item[0]))[:limit]"
    report_start = dedent(M7_REPORT).replace("@TOP@", top_start)
    report_fixed = dedent(M7_REPORT).replace("@TOP@", top_fixed)
    solution = {
        "logparse.py": dedent(M7_LOGPARSE_FIXED),
        "stats.py": dedent(M7_STATS_FIXED),
        "report.py": report_fixed,
    }
    starter = {
        "logparse.py": dedent(M7_LOGPARSE_START),
        "stats.py": dedent(M7_STATS_START),
        "report.py": report_start,
    }
    expected = run_golden(
        {**solution, **logs},
        "import json, report\nprint(json.dumps(report.build_report('logs')))\n")
    hidden = fill(M7_HIDDEN.lstrip("\n"), EXPECTED=literal(expected))
    return {
        "id": "m7-log-forensics",
        "prompt": (
            "report.py summarises the request logs under logs/ (gateway, billing and legacy services) but "
            "its numbers are wrong: several things in logparse.py, stats.py and report.py are off. "
            "Make the report correct. README.md states the rules the report must follow; the log "
            "formats themselves are not documented, so read the files under logs/ to see how each "
            "service writes a request, including multi-line entries, time zones, units, repeated request "
            "ids and cut-off final lines. Keep the module APIs described in README.md. Finish by "
            "running python -m pytest -q tests/test_public_logs.py."),
        "allowed": ["logparse.py", "stats.py", "report.py", "README.md", "tests/test_public_logs.py"],
        "public_verification": "python -m pytest -q tests/test_public_logs.py",
        "repo": {**starter, **logs, "README.md": dedent(M7_README), "tests/test_public_logs.py": M7_PUBLIC},
        "hidden": {"test_hidden_logs.py": hidden},
        "solution": solution,
    }


TASK_SPECS.append(build_m7())


# Each large fixture must stay big enough to force a long session at a 64k compaction threshold.
LARGE_SIZE_FLOORS = {
    "m5-wide-migration": 160_000,
    "m6-requirements-ledger": 160_000,
    "m7-log-forensics": 160_000,
}


def repo_bytes(spec: dict[str, object]) -> int:
    return sum(len(textwrap.dedent(value).lstrip("\n").encode("utf-8")) for value in dict(spec["repo"]).values())


def check_size_floors(specs: list[dict[str, object]] = None) -> dict[str, int]:
    """Raise if a long fixture shrank below its floor; return the sizes."""
    sizes = {}
    for spec in specs or TASK_SPECS:
        floor = LARGE_SIZE_FLOORS.get(str(spec["id"]))
        if floor is None:
            continue
        sizes[str(spec["id"])] = repo_bytes(spec)
        if sizes[str(spec["id"])] < floor:
            raise ValueError(f"{spec['id']} is {sizes[str(spec['id'])]} bytes, below its {floor} byte floor")
    return sizes


# --- m5-m7 builders end ---


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
        "hash_order": "relative-posix-codepoint-v1",
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
