"""Generate the v2 large-task fixtures (m1-m4) and `large_manifest_v2.json`.

v2 repairs the v1 grader contracts documented in
docs/perf/harness-gap/large-task-contracts.md. v1 (`make_large_tasks.py`,
`large_manifest.json`) is frozen and untouched; v2 tasks have their own IDs and
manifest, and their results are never pooled with v1.

The starter repositories and public tests are v1's, byte for byte: v2 changes
only the prompts, the hidden tests and the reference solutions. Every hidden
test traces to a prompt clause or to starter behavior (`REQUIREMENTS`), and
`VALIDATION` records how each starter, reference, alternative and deliberately
broken implementation must fare against the hidden tests.

These fixtures are public. They are not a protected holdout.
"""
from __future__ import annotations

import copy
import json
from pathlib import Path

import make_large_tasks as v1

ROOT = Path(__file__).resolve().parent
MANIFEST = ROOT / "large_manifest_v2.json"
VERSION = "v2"
NEW_TESTS = ["tests/test_*.py"]
V1 = {spec["id"]: spec for spec in v1.TASK_SPECS}


def starter(task):
    return copy.deepcopy(V1[task]["repo"])


M1_PROMPT = (
    "Implement the configuration loader used by the deployment CLI. "
    "parse_config(text) parses UTF-8 key=value text: ignore one leading byte-order mark (U+FEFF); "
    "skip blank lines and lines whose first non-whitespace character is #; "
    "trim whitespace around keys and values; "
    "split each line at the first = only, so values may contain =; "
    "the last duplicate key wins; "
    "raise ValueError for a line without = or with an empty key. "
    "load_config(defaults, text, overrides=None) returns a new dict: defaults, then file values, then overrides, "
    "and never mutates defaults or overrides. "
    "Keep read_config as an alias of parse_config. "
    "python config_cli.py FILE prints one key=value line per key, sorted by key. "
    "The existing public tests must pass; you may add tests under tests/."
)

M2_PROMPT = (
    "Migrate userlib's public formatter from format_user(name, active=True) to "
    "render_user(name, *, active=True, uppercase=False). "
    "render_user strips surrounding whitespace from the name; "
    "it raises ValueError if the name is not a string or is empty after stripping; "
    "uppercase=True uppercases the name only; "
    "it returns \"NAME (active)\" or \"NAME (inactive)\". "
    "Keep userlib.core.format_user(name, active=True) as a compatibility shim that returns "
    "render_user(name, active=active). "
    "The userlib package exports only render_user: __all__ == [\"render_user\"] and userlib has no "
    "format_user attribute. "
    "No module in userlib/ other than core.py, and not cli.py, may reference format_user; they call render_user. "
    "Update the README example to render_user. "
    "The public tests must pass: update tests/test_public_api.py for the new API; you may add tests under tests/."
)

M3_PROMPT = (
    "Fix the counter service across state_core and state_persistence. "
    "CounterService.apply(scope, request_id, delta) raises ValueError unless scope and request_id are "
    "non-empty strings and delta is a non-empty dict whose keys are non-empty strings and whose values "
    "are integers. "
    "It adds each delta value to that scope's counters and returns the scope's counters. "
    "It is idempotent per scope: repeating a request_id already applied in the same scope returns the "
    "current counters and changes neither counters nor history; the same request_id in another scope is "
    "a separate request. "
    "Each applied request appends {\"scope\": ..., \"request_id\": ...} to the stored state's history list, "
    "in apply order. "
    "It never mutates the caller's delta. "
    "Persist atomically: write the new state to a temporary file and then replace the state file, so the "
    "previous file stays intact if writing or replacing fails; when persistence fails, apply lets the "
    "OSError propagate. "
    "A new CounterService over the same file loads the same state. "
    "The existing public tests must pass; you may add tests under tests/."
)

M4_PROMPT = (
    "Add the inventory report feature to cli.py. "
    "Preserve the legacy command exactly: total NUMBER ... prints the sum of its arguments with two "
    "decimals, without tax. "
    "Add report NAME=PRICE:QTY ... . "
    "TOTAL for a record is PRICE * QTY * (1 + TAX / 100). "
    "--format text (the default) prints one line per record: NAME QTY TOTAL, with TOTAL to two decimals. "
    "--format json prints a JSON list with one object per record holding name, quantity and total. "
    "--sort name (the default) orders records by name; --sort total orders by total, highest first, "
    "ties by name. "
    "--tax PERCENT sets TAX; without it TAX comes from the INVENTORY_TAX environment variable, or 0 when "
    "it is unset. "
    "python cli.py must exit with status 2 for report without records, a record that is not "
    "NAME=PRICE:QTY, a negative price or quantity, or a --tax value that is not a non-negative number. "
    "Successful commands exit with status 0. "
    "The existing public tests must pass; you may add tests under tests/."
)


# Shared by hidden tests that run the public tests: a nested pytest in the
# workspace root, which is where the grader copies the hidden files.
PUBLIC = r'''
def test_public_tests_pass():
    root = pathlib.Path(__file__).resolve().parent
    result = subprocess.run([sys.executable, "-m", "pytest", "-q", "-p", "no:cacheprovider", PUBLIC_TEST],
                            cwd=root, capture_output=True, text=True, timeout=120)
    assert result.returncode == 0, result.stdout[-2000:]
'''

M1_HIDDEN = r'''
import pathlib
import subprocess
import sys

import pytest

import config_loader
import config_parser

PUBLIC_TEST = "tests/test_public_config.py"


def parse_config(text):
    return config_parser.parse_config(text)


def load_config(*args):
    return config_loader.load_config(*args)


def test_leading_bom_is_ignored():
    assert parse_config("\ufeffhost=a\n") == {"host": "a"}


def test_blank_and_comment_lines_are_skipped():
    assert parse_config("\n# note\n   # indented note\nhost=a\n\n") == {"host": "a"}


def test_keys_and_values_are_trimmed():
    assert parse_config("  host  =  local  \n") == {"host": "local"}


def test_value_keeps_equals_after_first_separator():
    assert parse_config("token=a=b=c\n") == {"token": "a=b=c"}


def test_last_duplicate_wins():
    assert parse_config("host=a\nhost=b\n") == {"host": "b"}


@pytest.mark.parametrize("text", ["broken", "host\n"])
def test_line_without_separator_raises(text):
    with pytest.raises(ValueError):
        parse_config(text)


@pytest.mark.parametrize("text", ["=value", "   = value"])
def test_empty_key_raises(text):
    with pytest.raises(ValueError):
        parse_config(text)


def test_read_config_is_an_alias():
    text = "host=a\nport=1\n"
    assert config_parser.read_config(text) == parse_config(text)


def test_load_config_precedence():
    assert load_config({"host": "d", "port": "80", "mode": "x"}, "port=443\nhost=f\n", {"host": "o"}) == {
        "host": "o", "port": "443", "mode": "x"}


def test_load_config_overrides_are_optional():
    assert load_config({"host": "d"}, "") == {"host": "d"}


def test_load_config_returns_a_new_dict_and_mutates_nothing():
    defaults, overrides = {"host": "d"}, {"port": "1"}
    result = load_config(defaults, "mode=x\n", overrides)
    assert result == {"host": "d", "mode": "x", "port": "1"}
    assert defaults == {"host": "d"} and overrides == {"port": "1"}
    assert result is not defaults and result is not overrides


def test_cli_prints_keys_sorted(tmp_path):
    path = tmp_path / "app.conf"
    path.write_text("zeta=1\n# note\nalpha = 2\n", encoding="utf-8")
    root = pathlib.Path(__file__).resolve().parent
    result = subprocess.run([sys.executable, "config_cli.py", str(path)], cwd=root,
                            capture_output=True, text=True, timeout=60)
    assert result.returncode == 0
    assert result.stdout.splitlines() == ["alpha=2", "zeta=1"]
''' + PUBLIC

M1_SOLUTION = {
    "config_parser.py": r'''
        def parse_config(text):
            if not isinstance(text, str):
                raise ValueError("configuration must be text")
            if text.startswith("\ufeff"):
                text = text[1:]
            result = {}
            for raw in text.splitlines():
                line = raw.strip()
                if not line or line.startswith("#"):
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
    ''',
    "config_loader.py": r'''
        from config_parser import parse_config


        def load_config(defaults, text, overrides=None):
            result = dict(defaults)
            result.update(parse_config(text))
            result.update(overrides or {})
            return result
    ''',
    "config_cli.py": r'''
        import sys

        from config_parser import parse_config


        def main(argv=None):
            path = (argv if argv is not None else sys.argv[1:])[0]
            with open(path, encoding="utf-8") as handle:
                values = parse_config(handle.read())
            for key in sorted(values):
                print(f"{key}={values[key]}")
            return 0


        if __name__ == "__main__":
            raise SystemExit(main())
    ''',
}

M2_HIDDEN = r'''
import pathlib
import re
import subprocess
import sys

import pytest

import userlib
from userlib import api, cli_helpers, core, exports, notifications, reports

PUBLIC_TEST = "tests/test_public_api.py"
ROOT = pathlib.Path(__file__).resolve().parent


def test_render_user_default_and_inactive():
    assert core.render_user("Ada") == "Ada (active)"
    assert core.render_user("Ada", active=False) == "Ada (inactive)"


def test_render_user_options_are_keyword_only():
    with pytest.raises(TypeError):
        core.render_user("Ada", False)


def test_render_user_strips_the_name():
    assert core.render_user("  Ada ") == "Ada (active)"


def test_uppercase_applies_to_the_name_only():
    assert core.render_user("Ada", uppercase=True) == "ADA (active)"
    assert core.render_user(" Ada ", active=False, uppercase=True) == "ADA (inactive)"


@pytest.mark.parametrize("name", ["", "   ", None, 7])
def test_invalid_names_raise(name):
    with pytest.raises(ValueError):
        core.render_user(name)


def test_format_user_shim_delegates():
    assert core.format_user("Ada") == core.render_user("Ada")
    assert core.format_user("Ada", False) == "Ada (inactive)"
    assert core.format_user("Ada", active=False) == "Ada (inactive)"


def test_package_exports_only_render_user():
    assert userlib.__all__ == ["render_user"]
    assert not hasattr(userlib, "format_user")
    assert userlib.render_user is core.render_user


def test_callers_keep_their_output():
    assert exports.export_user("Ada") == "Ada (active)"
    assert reports.report_user("Ada", False) == "Ada (inactive)"
    assert notifications.notification("Ada") == "User: Ada (active)"
    assert cli_helpers.cli_label("Ada") == "Ada (active)"
    assert api.api_payload("Ada") == {"label": "Ada (active)"}


def test_no_caller_references_format_user():
    paths = [p for p in (ROOT / "userlib").glob("*.py") if p.name != "core.py"] + [ROOT / "cli.py"]
    offenders = [p.name for p in paths if re.search(r"\bformat_user\b", p.read_text(encoding="utf-8"))]
    assert offenders == []


def test_cli_uses_render_user():
    result = subprocess.run([sys.executable, "cli.py", "  Ada "], cwd=ROOT, capture_output=True, text=True,
                            timeout=60)
    assert result.returncode == 0
    assert result.stdout == "Ada (active)\n"


def test_readme_example_uses_render_user():
    text = (ROOT / "README.md").read_text(encoding="utf-8")
    assert "render_user(" in text
''' + PUBLIC

M2_SOLUTION = {
    "README.md": r'''
        # User library

        `render_user(name, *, active=True, uppercase=False)` renders a display label.
        Old code can call `userlib.core.format_user(name, active=True)`, a compatibility shim.
    ''',
    "userlib/__init__.py": r'''
        from .core import render_user

        __all__ = ["render_user"]
    ''',
    "userlib/core.py": r'''
        def render_user(name, *, active=True, uppercase=False):
            if not isinstance(name, str) or not name.strip():
                raise ValueError("name is required")
            label = name.strip()
            if uppercase:
                label = label.upper()
            return f"{label} ({'active' if active else 'inactive'})"


        def format_user(name, active=True):
            """Compatibility shim for callers that have not migrated yet."""
            return render_user(name, active=active)
    ''',
    "userlib/exports.py": r'''
        from .core import render_user


        def export_user(name):
            return render_user(name)
    ''',
    "userlib/reports.py": r'''
        from .core import render_user


        def report_user(name, active=True):
            return render_user(name, active=active)
    ''',
    "userlib/notifications.py": r'''
        from .core import render_user


        def notification(name):
            return "User: " + render_user(name)
    ''',
    "userlib/cli_helpers.py": r'''
        from .core import render_user


        def cli_label(name):
            return render_user(name)
    ''',
    "userlib/api.py": r'''
        from .core import render_user


        def api_payload(name):
            return {"label": render_user(name)}
    ''',
    "cli.py": r'''
        import sys

        from userlib.core import render_user


        def main(argv=None):
            values = argv if argv is not None else sys.argv[1:]
            print(render_user(values[0] if values else "CLI"))
            return 0


        if __name__ == "__main__":
            raise SystemExit(main())
    ''',
    "tests/test_public_api.py": r'''
        from userlib import render_user
        from userlib.core import format_user
        from userlib.reports import report_user


        def test_render_api():
            assert render_user("Ada") == "Ada (active)"
            assert report_user("Ada", False) == "Ada (inactive)"


        def test_compatibility_shim():
            assert format_user("Ada") == "Ada (active)"
    ''',
}

M3_HIDDEN = r'''
import _io
import builtins
import io
import os
import pathlib
import subprocess
import sys

import pytest

from state_core import CounterService
from state_persistence import JsonStore

PUBLIC_TEST = "tests/test_public_state.py"


def service(tmp_path):
    return CounterService(JsonStore(pathlib.Path(tmp_path) / "state.json"))


def history(tmp_path):
    return [(e["scope"], e["request_id"]) for e in service(tmp_path).store.load()["history"]]


def test_apply_accumulates_and_returns_counters(tmp_path):
    s = service(tmp_path)
    assert s.apply("a", "r1", {"n": 2, "m": 1}) == {"n": 2, "m": 1}
    assert s.apply("a", "r2", {"n": 3}) == {"n": 5, "m": 1}


def test_repeated_request_is_idempotent_within_scope(tmp_path):
    s = service(tmp_path)
    s.apply("a", "same", {"n": 2})
    assert s.apply("a", "same", {"n": 99}) == {"n": 2}
    assert s.current("a") == {"n": 2}
    assert history(tmp_path) == [("a", "same")]


def test_request_ids_are_scoped(tmp_path):
    s = service(tmp_path)
    s.apply("a", "same", {"n": 2})
    assert s.apply("b", "same", {"n": 5}) == {"n": 5}
    assert s.current("a") == {"n": 2}


def test_history_records_applied_requests_in_order(tmp_path):
    s = service(tmp_path)
    s.apply("a", "same", {"n": 2})
    s.apply("a", "same", {"n": 9})
    s.apply("b", "same", {"n": 5})
    s.apply("a", "next", {"n": 3})
    assert history(tmp_path) == [("a", "same"), ("b", "same"), ("a", "next")]


def test_restart_loads_the_same_state(tmp_path):
    service(tmp_path).apply("a", "r1", {"x": 4})
    restarted = service(tmp_path)
    assert restarted.current("a") == {"x": 4}
    assert restarted.apply("a", "r1", {"x": 9}) == {"x": 4}


def test_caller_delta_is_not_mutated(tmp_path):
    delta = {"x": 4}
    s = service(tmp_path)
    s.apply("a", "r1", delta)
    s.apply("a", "r2", delta)
    assert delta == {"x": 4}


@pytest.mark.parametrize("scope, request_id, delta", [
    ("", "r", {"x": 1}), ("a", "", {"x": 1}), (None, "r", {"x": 1}), ("a", None, {"x": 1}),
    ("a", "r", {}), ("a", "r", {"": 1}), ("a", "r", {"x": "1"}), ("a", "r", None),
], ids=["empty-scope", "empty-request", "scope-none", "request-none", "empty-delta", "empty-key",
        "text-value", "delta-none"])
def test_invalid_input_raises(tmp_path, scope, request_id, delta):
    with pytest.raises(ValueError):
        service(tmp_path).apply(scope, request_id, delta)


class FailingWrites:
    """A file opened for writing whose writes fail, as on a full disk."""

    def __init__(self, handle):
        self._handle = handle

    def write(self, *args, **kwargs):
        raise OSError("disk full")

    writelines = write

    def __getattr__(self, name):
        return getattr(self._handle, name)

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self._handle.close()
        return False


def fail_writes(monkeypatch):
    real = _io.open

    def opener(file, mode="r", *args, **kwargs):
        handle = real(file, mode, *args, **kwargs)
        return FailingWrites(handle) if set(mode) & set("wax+") else handle

    for module in (builtins, io, _io):
        monkeypatch.setattr(module, "open", opener)

    def write(*args, **kwargs):
        raise OSError("disk full")

    monkeypatch.setattr(os, "write", write)


def fail_replace(monkeypatch):
    def fail(*args, **kwargs):
        raise OSError("replace failed")

    monkeypatch.setattr(os, "replace", fail)
    monkeypatch.setattr(os, "rename", fail)


def assert_previous_state_kept(tmp_path, before):
    path = pathlib.Path(tmp_path) / "state.json"
    assert path.read_bytes() == before
    assert service(tmp_path).current("a") == {"x": 1}


def test_failed_write_keeps_the_previous_file(tmp_path, monkeypatch):
    s = service(tmp_path)
    s.apply("a", "r1", {"x": 1})
    before = (pathlib.Path(tmp_path) / "state.json").read_bytes()
    with monkeypatch.context() as patch:
        fail_writes(patch)
        with pytest.raises(OSError):
            s.apply("a", "r2", {"x": 2})
    assert_previous_state_kept(tmp_path, before)


def test_failed_replace_keeps_the_previous_file(tmp_path, monkeypatch):
    s = service(tmp_path)
    s.apply("a", "r1", {"x": 1})
    before = (pathlib.Path(tmp_path) / "state.json").read_bytes()
    with monkeypatch.context() as patch:
        fail_replace(patch)
        try:
            s.apply("a", "r2", {"x": 2})
        except OSError:
            failed = True
        else:
            failed = False
    if failed:
        assert_previous_state_kept(tmp_path, before)
    else:
        # No replace or rename was used, so nothing failed: the new state
        # must then be what was persisted. A direct write is caught by the
        # failed-write test instead.
        assert service(tmp_path).current("a") == {"x": 3}
''' + PUBLIC

M3_SOLUTION = {
    "state_core/__init__.py": "from .service import CounterService\n",
    "state_core/model.py": r'''
        def require_text(value, name):
            if not isinstance(value, str) or not value:
                raise ValueError(f"{name} must be a non-empty string")
            return value


        def normalize_delta(delta):
            if not isinstance(delta, dict) or not delta:
                raise ValueError("delta must be a non-empty dict")
            result = {}
            for key, amount in delta.items():
                if not isinstance(key, str) or not key or not isinstance(amount, int) or isinstance(amount, bool):
                    raise ValueError("delta keys must be non-empty strings and values integers")
                result[key] = amount
            return result
    ''',
    "state_core/service.py": r'''
        import copy

        from state_core.model import normalize_delta, require_text


        class CounterService:
            def __init__(self, store):
                self.store = store

            def apply(self, scope, request_id, delta):
                require_text(scope, "scope")
                require_text(request_id, "request_id")
                normalized = normalize_delta(delta)
                state = self.store.load()
                scopes = state.setdefault("scopes", {})
                requests = state.setdefault("requests", {})
                history = state.setdefault("history", [])
                if not isinstance(requests.get(scope), dict):
                    requests[scope] = {}
                if request_id in requests[scope]:
                    return dict(scopes.get(scope, {}))
                values = dict(scopes.get(scope, {}))
                for key, amount in normalized.items():
                    values[key] = values.get(key, 0) + amount
                candidate = copy.deepcopy(state)
                candidate["scopes"][scope] = values
                candidate["requests"][scope][request_id] = True
                candidate["history"].append({"scope": scope, "request_id": request_id})
                self.store.save(candidate)
                return dict(values)

            def current(self, scope):
                return dict(self.store.load().get("scopes", {}).get(scope, {}))
    ''',
    "state_persistence/__init__.py": "from .json_store import JsonStore\n",
    "state_persistence/transaction.py": r'''
        import os
        import tempfile


        def write_text(path, text):
            path.parent.mkdir(parents=True, exist_ok=True)
            handle, temporary = tempfile.mkstemp(prefix=path.name + ".", dir=path.parent)
            try:
                with os.fdopen(handle, "w", encoding="utf-8") as output:
                    output.write(text)
                    output.flush()
                    os.fsync(output.fileno())
                os.replace(temporary, path)
            except BaseException:
                try:
                    os.unlink(temporary)
                except OSError:
                    pass
                raise
    ''',
    "state_persistence/json_store.py": r'''
        import json

        from state_persistence.transaction import write_text


        class JsonStore:
            def __init__(self, path):
                self.path = path

            def load(self):
                if not self.path.exists():
                    return {"scopes": {}, "requests": {}, "history": []}
                return json.loads(self.path.read_text(encoding="utf-8"))

            def save(self, state):
                write_text(self.path, json.dumps(state, sort_keys=True))
    ''',
}

M4_HIDDEN = r'''
import json
import os
import pathlib
import subprocess
import sys

import pytest

PUBLIC_TEST = "tests/test_public_cli.py"
ROOT = pathlib.Path(__file__).resolve().parent


def cli(*args, tax=None):
    env = {k: v for k, v in os.environ.items() if k != "INVENTORY_TAX"}
    if tax is not None:
        env["INVENTORY_TAX"] = tax
    result = subprocess.run([sys.executable, "cli.py", *args], cwd=ROOT, env=env, capture_output=True,
                            text=True, timeout=60)
    return result.returncode, result.stdout


def test_legacy_total_prints_the_sum():
    assert cli("total", "10", "2", "5", "3") == (0, "20.00\n")


def test_legacy_total_ignores_tax():
    assert cli("total", "10", "2", tax="10") == (0, "12.00\n")


def test_report_text_is_the_default_and_sorted_by_name():
    assert cli("report", "Banana=2:1", "Apple=3:2") == (0, "Apple 2 6.00\nBanana 1 2.00\n")


def test_report_json():
    code, out = cli("report", "Banana=2:1", "Apple=3:2", "--format", "json")
    assert code == 0
    rows = json.loads(out)
    assert [(r["name"], r["quantity"]) for r in rows] == [("Apple", 2), ("Banana", 1)]
    assert [r["total"] for r in rows] == pytest.approx([6.0, 2.0])


def test_sort_by_total_is_descending_with_name_tie_break():
    code, out = cli("report", "Cherry=6:2", "Apple=10:1", "Banana=4:3", "--sort", "total")
    assert (code, out.splitlines()) == (0, ["Banana 3 12.00", "Cherry 2 12.00", "Apple 1 10.00"])


def test_tax_option():
    assert cli("report", "A=10:1", "--tax", "10") == (0, "A 1 11.00\n")


def test_tax_from_environment_and_option_override():
    assert cli("report", "A=10:1", tax="10") == (0, "A 1 11.00\n")
    assert cli("report", "A=10:1", "--tax", "0", tax="10") == (0, "A 1 10.00\n")


@pytest.mark.parametrize("args", [
    ["report"], ["report", "bad"], ["report", "A=1"], ["report", "A=x:1"],
    ["report", "A=-1:2"], ["report", "A=1:-2"],
    ["report", "A=1:2", "--tax", "bad"], ["report", "A=1:2", "--tax", "-5"],
], ids=["no-records", "no-separator", "no-quantity", "text-price", "negative-price", "negative-quantity",
        "text-tax", "negative-tax"])
def test_invalid_input_exits_two(args):
    code, _ = cli(*args)
    assert code == 2
''' + PUBLIC

M4_SOLUTION = {
    "inventory/__init__.py": "",
    "inventory/config.py": r'''
        import os


        def load_config():
            return {"tax": float(os.environ.get("INVENTORY_TAX", "0"))}
    ''',
    "inventory/args.py": r'''
        import argparse


        def nonnegative(text):
            try:
                value = float(text)
            except ValueError as error:
                raise argparse.ArgumentTypeError("must be a number") from error
            if value < 0:
                raise argparse.ArgumentTypeError("must be non-negative")
            return value


        def parser():
            result = argparse.ArgumentParser()
            result.add_argument("command", choices=["total", "report"])
            result.add_argument("values", nargs="+")
            result.add_argument("--format", choices=["text", "json"], default="text")
            result.add_argument("--sort", choices=["name", "total"], default="name")
            result.add_argument("--tax", type=nonnegative)
            return result
    ''',
    "inventory/domain.py": r'''
        def parse_record(value):
            try:
                name, numbers = value.split("=", 1)
                price, quantity = numbers.split(":", 1)
                price, quantity = float(price), int(quantity)
            except ValueError as error:
                raise ValueError("record must be NAME=PRICE:QTY") from error
            if not name or price < 0 or quantity < 0:
                raise ValueError("price and quantity must be non-negative")
            return name, price, quantity


        def report(records, tax=0, sort_by="name"):
            rows = []
            for record in records:
                name, price, quantity = parse_record(record)
                rows.append({"name": name, "quantity": quantity, "total": price * quantity * (1 + tax / 100)})
            if sort_by == "total":
                return sorted(rows, key=lambda row: (-row["total"], row["name"]))
            return sorted(rows, key=lambda row: row["name"])
    ''',
    "inventory/output.py": r'''
        import json


        def render(rows, fmt="text"):
            if fmt == "json":
                return json.dumps(rows)
            return "\n".join(f"{r['name']} {r['quantity']} {r['total']:.2f}" for r in rows)
    ''',
    "cli.py": r'''
        import sys

        from inventory.args import parser
        from inventory.config import load_config
        from inventory.domain import report
        from inventory.output import render


        def main(argv=None):
            arguments = parser()
            args = arguments.parse_args(argv)
            if args.command == "total":
                values = [float(v) for v in args.values]
                print(f"{sum(values):.2f}")
                return 0
            tax = load_config()["tax"] if args.tax is None else args.tax
            try:
                rows = report(args.values, tax, args.sort)
            except ValueError as error:
                arguments.error(str(error))
            print(render(rows, args.format))
            return 0


        if __name__ == "__main__":
            raise SystemExit(main())
    ''',
}


def hidden(text, public):
    return {f"test_hidden_{public}.py": text.lstrip("\n")}


TASK_SPECS = [
    {"id": "m1-config-parser-v2", "base": "m1-config-parser", "prompt": M1_PROMPT,
     "hidden": hidden(M1_HIDDEN, "config"), "solution": M1_SOLUTION},
    {"id": "m2-public-api-migration-v2", "base": "m2-public-api-migration", "prompt": M2_PROMPT,
     "hidden": hidden(M2_HIDDEN, "api"), "solution": M2_SOLUTION},
    {"id": "m3-state-persistence-v2", "base": "m3-state-persistence", "prompt": M3_PROMPT,
     "hidden": hidden(M3_HIDDEN, "state"), "solution": M3_SOLUTION},
    {"id": "m4-cli-feature-v2", "base": "m4-cli-feature", "prompt": M4_PROMPT,
     "hidden": hidden(M4_HIDDEN, "cli"), "solution": M4_SOLUTION},
]
for _spec in TASK_SPECS:
    _base = V1[_spec["base"]]
    _spec.update(repo=starter(_spec["base"]), allowed=list(_base["allowed"]),
                 public_verification=_base["public_verification"])
    # The reference keeps every starter file it does not replace.
    _spec["solution"] = {**{k: v for k, v in _spec["repo"].items() if k.startswith("tests/")}, **_spec["solution"]}

# Hidden test -> (requirement, source). The source is a phrase that appears
# verbatim in the task prompt, or "starter:" and the starter behavior it keeps.
REQUIREMENTS = {
    "m1-config-parser-v2": {
        "test_leading_bom_is_ignored": ("m1.bom", "ignore one leading byte-order mark (U+FEFF)"),
        "test_blank_and_comment_lines_are_skipped":
            ("m1.comments", "skip blank lines and lines whose first non-whitespace character is #"),
        "test_keys_and_values_are_trimmed": ("m1.trim", "trim whitespace around keys and values"),
        "test_value_keeps_equals_after_first_separator":
            ("m1.first_separator", "split each line at the first = only, so values may contain ="),
        "test_last_duplicate_wins": ("m1.last_wins", "the last duplicate key wins"),
        "test_line_without_separator_raises": ("m1.no_separator", "raise ValueError for a line without ="),
        "test_empty_key_raises": ("m1.empty_key", "or with an empty key"),
        "test_read_config_is_an_alias": ("m1.alias", "Keep read_config as an alias of parse_config"),
        "test_load_config_precedence": ("m1.precedence", "defaults, then file values, then overrides"),
        "test_load_config_overrides_are_optional": ("m1.optional_overrides", "load_config(defaults, text, overrides=None)"),
        "test_load_config_returns_a_new_dict_and_mutates_nothing":
            ("m1.no_mutation", "returns a new dict"),
        "test_cli_prints_keys_sorted": ("m1.cli_sorted", "prints one key=value line per key, sorted by key"),
        "test_public_tests_pass": ("m1.public_tests", "The existing public tests must pass"),
    },
    "m2-public-api-migration-v2": {
        "test_render_user_default_and_inactive": ("m2.render_states", "returns \"NAME (active)\" or \"NAME (inactive)\""),
        "test_render_user_options_are_keyword_only":
            ("m2.keyword_only", "render_user(name, *, active=True, uppercase=False)"),
        "test_render_user_strips_the_name": ("m2.strip", "render_user strips surrounding whitespace from the name"),
        "test_uppercase_applies_to_the_name_only": ("m2.uppercase", "uppercase=True uppercases the name only"),
        "test_invalid_names_raise":
            ("m2.invalid_names", "raises ValueError if the name is not a string or is empty after stripping"),
        "test_format_user_shim_delegates": ("m2.shim", "returns render_user(name, active=active)"),
        "test_package_exports_only_render_user": ("m2.exports", "__all__ == [\"render_user\"]"),
        "test_callers_keep_their_output": ("m2.caller_output", "starter: each caller's output for a plain name"),
        "test_no_caller_references_format_user":
            ("m2.no_stale_references", "No module in userlib/ other than core.py, and not cli.py, may reference format_user"),
        "test_cli_uses_render_user": ("m2.cli", "they call render_user"),
        "test_readme_example_uses_render_user": ("m2.readme", "Update the README example to render_user"),
        "test_public_tests_pass": ("m2.public_tests", "The public tests must pass"),
    },
    "m3-state-persistence-v2": {
        "test_apply_accumulates_and_returns_counters": ("m3.accumulate", "adds each delta value to that scope's counters and returns the scope's counters"),
        "test_repeated_request_is_idempotent_within_scope":
            ("m3.idempotent", "changes neither counters nor history"),
        "test_request_ids_are_scoped": ("m3.scoped_ids", "the same request_id in another scope is a separate request"),
        "test_history_records_applied_requests_in_order": ("m3.history", "in apply order"),
        "test_restart_loads_the_same_state": ("m3.restart", "A new CounterService over the same file loads the same state"),
        "test_caller_delta_is_not_mutated": ("m3.no_mutation", "It never mutates the caller's delta"),
        "test_invalid_input_raises": ("m3.validation", "raises ValueError unless scope and request_id are"),
        "test_failed_write_keeps_the_previous_file": ("m3.atomic", "the previous file stays intact if writing or replacing fails"),
        "test_failed_replace_keeps_the_previous_file": ("m3.atomic", "the previous file stays intact if writing or replacing fails"),
        "test_public_tests_pass": ("m3.public_tests", "The existing public tests must pass"),
    },
    "m4-cli-feature-v2": {
        "test_legacy_total_prints_the_sum": ("m4.legacy_total", "prints the sum of its arguments with two decimals"),
        "test_legacy_total_ignores_tax": ("m4.legacy_no_tax", "without tax"),
        "test_report_text_is_the_default_and_sorted_by_name":
            ("m4.text_default", "--format text (the default) prints one line per record: NAME QTY TOTAL"),
        "test_report_json": ("m4.json", "--format json prints a JSON list with one object per record"),
        "test_sort_by_total_is_descending_with_name_tie_break":
            ("m4.sort_total", "--sort total orders by total, highest first, ties by name"),
        "test_tax_option": ("m4.tax_option", "--tax PERCENT sets TAX"),
        "test_tax_from_environment_and_option_override":
            ("m4.tax_env", "without it TAX comes from the INVENTORY_TAX environment variable"),
        "test_invalid_input_exits_two": ("m4.exit_two", "python cli.py must exit with status 2"),
        "test_public_tests_pass": ("m4.public_tests", "The existing public tests must pass"),
    },
}


def patch(*pairs):
    """Variant: (file, old, new) replacements applied to the reference."""
    return {"patch": [tuple(pair) for pair in pairs]}


# How each implementation must fare against the hidden tests. "fails" lists
# the exact failing hidden test node names (without the file); anything else
# failing, or any listed test passing, is a validation error.
VALIDATION = {
    "m1-config-parser-v2": {
        "starter_fails": [
            "test_cli_prints_keys_sorted", "test_empty_key_raises[   = value]", "test_empty_key_raises[=value]",
            "test_keys_and_values_are_trimmed", "test_leading_bom_is_ignored",
            "test_load_config_precedence", "test_load_config_returns_a_new_dict_and_mutates_nothing",
            ],
        "alternatives": {
            "removeprefix BOM and regex parser": {"files": {"config_parser.py": r'''
                import re

                LINE = re.compile(r"^\s*([^=]*?)\s*=\s*(.*?)\s*$")


                def parse_config(text):
                    result = {}
                    for raw in text.removeprefix("\ufeff").splitlines():
                        if not raw.strip() or raw.lstrip().startswith("#"):
                            continue
                        match = LINE.match(raw)
                        if match is None or not match.group(1):
                            raise ValueError(f"bad line: {raw!r}")
                        result[match.group(1)] = match.group(2)
                    return result


                read_config = parse_config
            '''}},
            "loader copies with a dict merge": patch(
                ("config_loader.py", "result = dict(defaults)\n    result.update(parse_config(text))\n    result.update(overrides or {})\n    return result",
                 "return {**defaults, **parse_config(text), **(overrides or {})}")),
        },
        "broken": {
            "BOM kept": (patch(("config_parser.py", "text = text[1:]", "pass")),
                         ["test_leading_bom_is_ignored"]),
            "split at the last =": (patch(("config_parser.py", 'line.split("=", 1)', 'line.rsplit("=", 1)')),
                                    ["test_value_keeps_equals_after_first_separator"]),
            "first duplicate wins": (patch(("config_parser.py", "result[key] = value", "result.setdefault(key, value)")),
                                     ["test_last_duplicate_wins"]),
            "defaults mutated": (patch(("config_loader.py", "result = dict(defaults)", "result = defaults")),
                                 ["test_load_config_returns_a_new_dict_and_mutates_nothing"]),
            "CLI in file order": (patch(("config_cli.py", "for key in sorted(values):", "for key in values:")),
                                  ["test_cli_prints_keys_sorted"]),
            "comment lines parsed": (patch(("config_parser.py", 'if not line or line.startswith("#"):', "if not line:")),
                                     # the CLI test's file has a comment line too
                                     ["test_blank_and_comment_lines_are_skipped", "test_cli_prints_keys_sorted"]),
            "lines without = skipped": (patch(("config_parser.py", 'raise ValueError("expected key=value")', "continue")),
                                        ["test_line_without_separator_raises[broken]",
                                         "test_line_without_separator_raises[host\\n]"]),
            "alias removed": (patch(("config_parser.py", "def read_config(text):\n    return parse_config(text)",
                                     "def load_text(text):\n    return parse_config(text)")),
                              ["test_public_tests_pass", "test_read_config_is_an_alias"]),
            "overrides required": (patch(("config_loader.py", "result.update(overrides or {})", "result.update(overrides)")),
                                   # the public test calls load_config without overrides too
                                   ["test_load_config_overrides_are_optional", "test_public_tests_pass"]),
        },
    },
    "m2-public-api-migration-v2": {
        # Everything the migration adds; caller output and the public tests already pass.
        "starter_fails": [
            "test_cli_uses_render_user", "test_format_user_shim_delegates", "test_invalid_names_raise[   ]",
            "test_invalid_names_raise[7]", "test_invalid_names_raise[None]", "test_invalid_names_raise[]",
            "test_no_caller_references_format_user", "test_package_exports_only_render_user",
            "test_readme_example_uses_render_user", "test_render_user_default_and_inactive",
            "test_render_user_options_are_keyword_only", "test_render_user_strips_the_name",
            "test_uppercase_applies_to_the_name_only"],
        "alternatives": {
            "shim forwards keywords": patch(("userlib/core.py", "def format_user(name, active=True):",
                                             "def format_user(name, active=True, **_ignored):")),
            "render_user in its own module": {"files": {
                "userlib/render.py": r'''
                    def render_user(name, *, active=True, uppercase=False):
                        if not isinstance(name, str) or not name.strip():
                            raise ValueError("name is required")
                        label = name.strip().upper() if uppercase else name.strip()
                        return f"{label} ({'active' if active else 'inactive'})"
                ''',
                "userlib/core.py": r'''
                    from .render import render_user


                    def format_user(name, active=True):
                        return render_user(name, active=active)
                ''',
            }},
        },
        "broken": {
            "name not stripped": (patch(("userlib/core.py", "label = name.strip()", "label = name")),
                                  ["test_cli_uses_render_user", "test_render_user_strips_the_name",
                                   "test_uppercase_applies_to_the_name_only"]),
            "whole label uppercased": (patch(("userlib/core.py",
                                              "return f\"{label} ({'active' if active else 'inactive'})\"",
                                              "text = f\"{label} ({'active' if active else 'inactive'})\"\n"
                                              "    return text.upper() if uppercase else text")),
                                       ["test_uppercase_applies_to_the_name_only"]),
            "old name re-exported": (patch(("userlib/__init__.py", "from .core import render_user",
                                            "from .core import format_user, render_user")),
                                     # __init__.py is in userlib/, so the re-export also references it.
                                     ["test_no_caller_references_format_user", "test_package_exports_only_render_user"]),
            "caller still on the shim": (patch(("userlib/exports.py", "from .core import render_user",
                                                "from .core import format_user as render_user")),
                                         ["test_no_caller_references_format_user"]),
            "options positional": (patch(("userlib/core.py", "def render_user(name, *, active=True, uppercase=False):",
                                          "def render_user(name, active=True, uppercase=False):")),
                                   ["test_render_user_options_are_keyword_only"]),
            "caller output changed": (patch(("userlib/notifications.py", '"User: "', '"user: "')),
                                      ["test_callers_keep_their_output"]),
            "public test not migrated": ({"files": {"tests/test_public_api.py": V1["m2-public-api-migration"]["repo"]["tests/test_public_api.py"]}},
                                         ["test_public_tests_pass"]),
        },
    },
    "m3-state-persistence-v2": {
        # The starter writes the file directly: the replace fault never fires, the write fault catches it.
        "starter_fails": [
            "test_failed_write_keeps_the_previous_file",
            "test_history_records_applied_requests_in_order",
            "test_invalid_input_raises[delta-none]", "test_invalid_input_raises[empty-delta]",
            "test_invalid_input_raises[empty-key]", "test_invalid_input_raises[empty-request]",
            "test_invalid_input_raises[empty-scope]", "test_invalid_input_raises[request-none]",
            "test_invalid_input_raises[scope-none]", "test_invalid_input_raises[text-value]",
            "test_repeated_request_is_idempotent_within_scope", "test_restart_loads_the_same_state"],
        "alternatives": {
            "NamedTemporaryFile and Path.replace in json_store": {"files": {
                "state_persistence/json_store.py": r'''
                    import json
                    import tempfile
                    from pathlib import Path


                    class JsonStore:
                        def __init__(self, path):
                            self.path = path

                        def load(self):
                            if not self.path.exists():
                                return {"scopes": {}, "requests": {}, "history": []}
                            return json.loads(self.path.read_text(encoding="utf-8"))

                        def save(self, state):
                            with tempfile.NamedTemporaryFile("w", encoding="utf-8", dir=self.path.parent,
                                                             delete=False, suffix=".tmp") as handle:
                                json.dump(state, handle)
                            try:
                                Path(handle.name).replace(self.path)
                            except OSError:
                                Path(handle.name).unlink(missing_ok=True)
                                raise
                ''',
            }},
            "sibling file written with Path.write_text then os.replace": patch(
                ("state_persistence/transaction.py",
                 'handle, temporary = tempfile.mkstemp(prefix=path.name + ".", dir=path.parent)\n    try:\n        with os.fdopen(handle, "w", encoding="utf-8") as output:\n            output.write(text)\n            output.flush()\n            os.fsync(output.fileno())\n        os.replace(temporary, path)',
                 'temporary = path.with_name(path.name + ".tmp")\n    try:\n        temporary.write_text(text, encoding="utf-8")\n        os.replace(temporary, path)')),
        },
        "broken": {
            "direct write": (patch(("state_persistence/json_store.py",
                                    "write_text(self.path, json.dumps(state, sort_keys=True))",
                                    'self.path.write_text(json.dumps(state, sort_keys=True), encoding="utf-8")')),
                             ["test_failed_write_keeps_the_previous_file"]),
            "old file removed before replace": (patch(("state_persistence/transaction.py",
                                                       "os.replace(temporary, path)",
                                                       "if path.exists():\n            os.unlink(path)\n        os.replace(temporary, path)")),
                                                ["test_failed_replace_keeps_the_previous_file"]),
            "persistence errors swallowed": (patch(("state_persistence/json_store.py",
                                                    "write_text(self.path, json.dumps(state, sort_keys=True))",
                                                    "try:\n            write_text(self.path, json.dumps(state, sort_keys=True))\n        except OSError:\n            pass")),
                                             ["test_failed_replace_keeps_the_previous_file",
                                              "test_failed_write_keeps_the_previous_file"]),
            "empty delta accepted": (patch(("state_core/model.py", "if not isinstance(delta, dict) or not delta:",
                                            "if not isinstance(delta, dict):")),
                                     ["test_invalid_input_raises[empty-delta]"]),
            "counters overwritten": (patch(("state_core/service.py", "values[key] = values.get(key, 0) + amount",
                                            "values[key] = amount")),
                                     ["test_apply_accumulates_and_returns_counters"]),
            "request ids shared across scopes": (patch(("state_core/service.py", "if request_id in requests[scope]:",
                                                        "if any(request_id in seen for seen in requests.values()):")),
                                                 # the history test reuses an ID in another scope too
                                                 ["test_history_records_applied_requests_in_order",
                                                  "test_request_ids_are_scoped"]),
            "caller delta consumed": (patch(("state_core/model.py", "        result[key] = amount\n    return result",
                                             "        result[key] = amount\n    delta.clear()\n    return result")),
                                      ["test_caller_delta_is_not_mutated"]),
            "public result shape changed": (patch(("state_core/service.py", "        self.store.save(candidate)\n        return dict(values)",
                                                   "        self.store.save(candidate)\n        return None")),
                                            ["test_apply_accumulates_and_returns_counters", "test_public_tests_pass",
                                             "test_request_ids_are_scoped"]),
            "repeat applied again": (patch(("state_core/service.py", "if request_id in requests[scope]:",
                                            "if False:")),
                                     ["test_history_records_applied_requests_in_order",
                                      "test_repeated_request_is_idempotent_within_scope",
                                      "test_restart_loads_the_same_state"]),
        },
    },
    "m4-cli-feature-v2": {
        # argparse already exits 2 for no records and for the unknown --tax option.
        "starter_fails": [
            "test_invalid_input_exits_two[negative-price]", "test_invalid_input_exits_two[negative-quantity]",
            "test_invalid_input_exits_two[no-quantity]",
            "test_invalid_input_exits_two[no-separator]", "test_invalid_input_exits_two[text-price]",
            "test_report_json", "test_report_text_is_the_default_and_sorted_by_name",
            "test_sort_by_total_is_descending_with_name_tie_break", "test_tax_from_environment_and_option_override",
            "test_tax_option"],
        "alternatives": {
            "main returns 2": patch(("cli.py", "arguments.error(str(error))",
                                     'print(f"error: {error}", file=sys.stderr)\n        return 2')),
            "sys.exit(2)": patch(("cli.py", "arguments.error(str(error))",
                                  'print(f"error: {error}", file=sys.stderr)\n        sys.exit(2)')),
            "raise SystemExit(2)": patch(("cli.py", "arguments.error(str(error))", "raise SystemExit(2)")),
        },
        "broken": {
            "invalid input exits 1": (patch(("cli.py", "arguments.error(str(error))", "return 1")),
                                      ["test_invalid_input_exits_two[negative-price]",
                                       "test_invalid_input_exits_two[negative-quantity]",
                                       "test_invalid_input_exits_two[no-quantity]",
                                       "test_invalid_input_exits_two[no-separator]",
                                       "test_invalid_input_exits_two[text-price]"]),
            "legacy total multiplies pairs": (patch(("cli.py", 'print(f"{sum(values):.2f}")',
                                                     'print(f"{sum(a * b for a, b in zip(values[::2], values[1::2])):.2f}")')),
                                              ["test_legacy_total_ignores_tax", "test_legacy_total_prints_the_sum",
                                               "test_public_tests_pass"]),
            "tax applied to legacy total": (patch(("cli.py", 'print(f"{sum(values):.2f}")',
                                                   'print(f"{sum(values) * (1 + load_config()[\'tax\'] / 100):.2f}")')),
                                            ["test_legacy_total_ignores_tax"]),
            "ties by reverse name": (patch(("inventory/domain.py", 'key=lambda row: (-row["total"], row["name"])',
                                            'key=lambda row: (row["total"], row["name"]), reverse=True')),
                                     ["test_sort_by_total_is_descending_with_name_tie_break"]),
        },
    },
}


def main() -> None:
    metadata = []
    for spec in TASK_SPECS:
        entry = v1.write_task(spec)
        task = json.loads((v1.TASK_ROOT / spec["id"] / "task.json").read_text(encoding="utf-8"))
        task.update(fixture_version=VERSION, base_task=spec["base"], allowed_patterns=NEW_TESTS)
        v1.write_json(v1.TASK_ROOT / spec["id"] / "task.json", task)
        metadata.append(entry)
    manifest = {
        "schema_version": 1,
        "hash_order": "relative-posix-codepoint-v1",
        "task_set": "large",
        "fixture_version": VERSION,
        "generator": "scripts/bench/make_large_tasks_v2.py",
        "independent_holdout": False,
        "tasks": [item["id"] for item in metadata],
        "fixtures": {
            item["id"]: {
                "task_set": item["task_set"],
                "allowed": item["allowed"],
                "allowed_patterns": NEW_TESTS,
                "public_verification": item["public_verification"],
                "public_hash": item["public_hash"],
                "reference_solution_hash": item["reference_solution_hash"],
                "hidden_grader_hash": item["hidden_grader_hash"],
            }
            for item in metadata
        },
    }
    v1.write_json(MANIFEST, manifest)
    print(json.dumps({"tasks": manifest["tasks"], "manifest": str(MANIFEST)}, indent=2))


if __name__ == "__main__":
    main()
