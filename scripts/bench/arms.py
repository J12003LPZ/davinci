"""Four-arm pilot layer (plan B03): arm table, settings, preflight, schedule, attribution.

Arms: ``davinci_direct``, ``davinci_script``, ``droid_direct``, ``droid_script``.
Pure and offline: nothing here starts a harness or touches a network. The legacy
harness names (``davinci``, ``codex``, ``droid``, ``droid-direct``, ``droid-script``)
and the three-arm Stage 0 study are untouched; an arm maps onto one of them.

    python arms.py plan --tasks T1 T2 --reps 2 --seed 7 [--arms ...]

prints the planned run count first so the run budget can be approved.

``davinci_script`` runs DaVinci in controlled Codemode, selected only by the trusted
``--codemode controlled`` flag plus a trusted settings block with absolute
``nodePath`` and ``hostPath`` (``codemode_host/config.rs``). Nothing in a task or a
prompt can change the mode. Its preflight must pass first; a failure raises
``PreflightError`` and never selects another arm or read-only.
"""
import argparse
import copy
import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path, PurePosixPath
from typing import NoReturn

from campaign import observed_mode, schedule
from droid_usage import normalize_mode

ARMS = ("davinci_direct", "davinci_script", "droid_direct", "droid_script")
ARM_SPECS = {
    "davinci_direct": {"family": "davinci", "harness": "davinci", "mode": "off"},
    "davinci_script": {"family": "davinci", "harness": "davinci", "mode": "controlled"},
    "droid_direct": {"family": "droid", "harness": "droid-direct", "mode": "direct-only"},
    "droid_script": {"family": "droid", "harness": "droid-script", "mode": "script-only"},
}
ADMITTED_NODE_VERSION = "v24.21.0"
# Mirrors HostAssets::validate in codemode_host/assets.rs.
MANIFEST_FIELDS = {"schema_version", "protocol_version", "node_version", "pi_version",
                   "quickjs_version", "entry", "worker", "wasm", "assets"}
MANIFEST_PINS = {"schema_version": 1, "protocol_version": 1, "node_version": "24.21.0",
                 "pi_version": "1.0.2", "quickjs_version": "3.6.2"}
MAX_ASSETS = 128
MAX_ASSET_BYTES = 64 * 1024 * 1024
TRUSTED_MANIFEST = (Path(__file__).resolve().parents[2] / "crates" / "davinci-coding-agent"
                    / "src" / "codemode_host" / "assets-manifest.json")


class PreflightError(RuntimeError):
    """An arm cannot run. The campaign stops; no other arm or mode is substituted."""


def spec(arm):
    if arm not in ARM_SPECS:
        raise ValueError("unknown arm " + repr(arm))
    return ARM_SPECS[arm]


def arm_settings(arm, base_settings, node_path=None, host_path=None):
    """Trusted DaVinci settings for an arm; the base settings are never modified.

    ``davinci_direct`` forces Codemode off. ``davinci_script`` sets controlled mode
    with the given absolute paths. Droid arms carry no DaVinci settings.
    """
    arm_spec = spec(arm)
    if arm_spec["family"] != "davinci":
        return None
    settings = copy.deepcopy(base_settings)
    if arm_spec["mode"] == "off":
        settings["codemode"] = {"mode": "off"}
        return settings
    for key, value in (("nodePath", node_path), ("hostPath", host_path)):
        if not isinstance(value, str) or not Path(value).is_absolute():
            raise ValueError(f"davinci_script needs an absolute {key}")
    settings["codemode"] = {"mode": "controlled", "nodePath": node_path, "hostPath": host_path}
    return settings


def davinci_session_flags(arm):
    """CLI flags selecting the arm's mode, as ``bench.command`` builds them.

    Controlled mode needs a session journal, so it never gets ``--no-session``.
    """
    arm_spec = spec(arm)
    if arm_spec["family"] != "davinci":
        raise ValueError("not a DaVinci arm")
    return ["--no-session"] if arm_spec["mode"] == "off" else ["--codemode", arm_spec["mode"]]


def _is_link(path):
    """Symlink or Windows reparse point (junction), as ``reject_link`` in assets.rs."""
    if os.path.islink(path):
        return True
    attributes = getattr(os.lstat(path), "st_file_attributes", 0)
    return bool(attributes & 0x400)


def _sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(16384), b""):
            digest.update(block)
    return digest.hexdigest()


def _fail(message) -> NoReturn:
    raise PreflightError(message)


def load_manifest(manifest_path=TRUSTED_MANIFEST):
    """The trusted manifest, checked against the same pins as the Rust build."""
    try:
        manifest = json.loads(Path(manifest_path).read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        _fail(f"host manifest unreadable: {type(error).__name__}")
    if not isinstance(manifest, dict) or set(manifest) != MANIFEST_FIELDS:
        _fail("host manifest has unexpected fields")
    for key, expected in MANIFEST_PINS.items():
        if manifest[key] != expected:
            _fail(f"unsupported host manifest {key}")
    assets = manifest["assets"]
    if not isinstance(assets, list) or not 0 < len(assets) <= MAX_ASSETS:
        _fail("unsupported host manifest asset list")
    for asset in assets:
        if (not isinstance(asset, dict) or set(asset) != {"path", "bytes", "sha256"}
                or not isinstance(asset["path"], str) or type(asset["bytes"]) is not int
                or not isinstance(asset["sha256"], str)):
            _fail("malformed host manifest asset")
    return manifest


def _asset_path(root, relative):
    parts = PurePosixPath(relative).parts
    if (not relative or "\\" in relative or ":" in relative
            or any(part in ("", ".", "..", "/") for part in parts) or PurePosixPath(relative).is_absolute()):
        _fail("invalid host asset path " + repr(relative))
    path = Path(root)
    for part in parts:
        path = path / part
        if not os.path.lexists(path) or _is_link(path):
            _fail("missing or linked host asset " + relative)
    return path


def validate_host_bundle(host_path, manifest):
    """Every manifest asset present with the right size and SHA-256, and nothing else."""
    root = Path(host_path)
    if not root.is_absolute():
        _fail("host path must be absolute")
    if not root.is_dir():
        _fail("host bundle directory is missing")
    for ancestor in [root, *root.parents]:
        if _is_link(ancestor):
            _fail("linked host bundle path is not admitted")
    listed = set()
    for asset in manifest["assets"]:
        relative = asset["path"]
        path = _asset_path(root, relative)
        if (relative in listed or asset["bytes"] > MAX_ASSET_BYTES or len(asset["sha256"]) != 64
                or not path.is_file() or path.stat().st_size != asset["bytes"]):
            _fail("invalid or missing host asset " + relative)
        if _sha256(path) != asset["sha256"]:
            _fail("host asset digest mismatch " + relative)
        listed.add(relative)
    present = set()
    for base, directories, names in os.walk(root, followlinks=False):
        for name in [*directories, *names]:
            path = Path(base) / name
            if _is_link(path):
                _fail("linked host asset " + path.relative_to(root).as_posix())
            if name in names:
                present.add(path.relative_to(root).as_posix())
    if present != listed:
        _fail("unlisted host assets: " + ", ".join(sorted(present - listed)))
    for required in (manifest["entry"], manifest["worker"], manifest["wasm"]):
        if required not in listed:
            _fail("unlisted host entry point")
    return {"files": len(listed)}


def probe_node_version(node_path):
    """``node --version`` output. Tests inject a fake instead of running a binary."""
    return subprocess.run([node_path, "--version"], capture_output=True, text=True,
                          timeout=30, check=True).stdout


def preflight(arm, node_path=None, host_path=None, *, manifest_path=TRUSTED_MANIFEST,
              node_probe=probe_node_version):
    """Check an arm can run. Returns a record for the row; raises ``PreflightError``.

    Only ``davinci_script`` has preconditions here. The Node version must be exactly
    24.21.0 (trailing line breaks only are trimmed, as in ``validate_node_version``).
    """
    arm_spec = spec(arm)
    if arm_spec["mode"] != "controlled":
        return {"arm": arm, "checked": False}
    for key, value in (("node", node_path), ("host", host_path)):
        if not isinstance(value, str) or not Path(value).is_absolute():
            _fail(f"{key} path must be absolute")
    if not Path(node_path).is_file():
        _fail("Node binary is missing at " + node_path)
    try:
        version = node_probe(node_path)
    except Exception as error:  # a failed probe is a failed preflight, whatever it raised
        _fail(f"Node version probe failed: {type(error).__name__}")
    if not isinstance(version, str) or version.rstrip("\r\n") != ADMITTED_NODE_VERSION:
        _fail(f"Node must be exactly {ADMITTED_NODE_VERSION}")
    manifest = load_manifest(manifest_path)
    bundle = validate_host_bundle(host_path, manifest)
    return {"arm": arm, "checked": True, "node_path": node_path, "node_version": ADMITTED_NODE_VERSION,
            "host_path": host_path, "host_files": bundle["files"],
            "manifest_sha256": _sha256(manifest_path)}


def schedule_arms(tasks, repetitions, arms, seed=0):
    """The existing counterbalanced schedule, over arm names: (arm, task, rep) tuples."""
    arms = list(arms)
    for arm in arms:
        spec(arm)
    return schedule(tasks, repetitions, arms, seed)


def plan(tasks, repetitions, arms=ARMS, seed=0):
    """The schedule plus its size, to approve before anything runs."""
    order = schedule_arms(tasks, repetitions, arms, seed)
    per_arm = {arm: sum(1 for item in order if item[0] == arm) for arm in arms}
    return {"planned_runs": len(order), "per_arm": per_arm, "seed": seed,
            "schedule": [list(item) for item in order]}


def stamp_row(row, arm, preflight_record=None):
    """Copy of a result row with the arm it was launched as and the mode it showed.

    ``arm_attribution`` is ``confirmed`` only when the row's harness matches the
    arm and the mode the live run reported equals the arm's mode; ``mismatch`` when
    either contradicts it; ``unknown`` when the run reported no mode. Only a
    ``confirmed`` row may be counted for the arm.
    """
    arm_spec = spec(arm)
    if arm_spec["family"] == "davinci":
        observed = observed_mode(row, "codemode")
        harness_ok = row.get("harness") == arm_spec["harness"]
    else:
        observed = normalize_mode(row.get("droid_resolved_tool_execution_mode"))
        harness_ok = (row.get("harness") == arm_spec["harness"]
                      or (row.get("harness") == "droid"
                          and normalize_mode(row.get("droid_tool_execution_mode")) == arm_spec["mode"]))
    if not harness_ok or (observed is not None and observed != arm_spec["mode"]):
        attribution = "mismatch"
    elif observed is None:
        attribution = "unknown"
    else:
        attribution = "confirmed"
    stamped = dict(row)
    stamped.update(arm=arm, arm_requested_mode=arm_spec["mode"], arm_observed_mode=observed,
                   arm_attribution=attribution, arm_preflight=preflight_record)
    return stamped


def row_arm(row):
    """The arm a row may be counted for, or None when it is unattributed or contradicted."""
    return row.get("arm") if row.get("arm_attribution") == "confirmed" else None


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = ap.add_subparsers(dest="command", required=True)
    plan_parser = sub.add_parser("plan", help="print the planned run count and schedule; runs nothing")
    plan_parser.add_argument("--tasks", nargs="+", required=True)
    plan_parser.add_argument("--reps", type=int, default=1)
    plan_parser.add_argument("--rep-start", type=int, default=0)
    plan_parser.add_argument("--seed", type=int, default=0)
    plan_parser.add_argument("--arms", nargs="+", choices=ARMS, default=list(ARMS))
    args = ap.parse_args(argv)
    if args.reps < 1 or args.rep_start < 0:
        ap.error("reps must be positive and rep-start nonnegative")
    result = plan(args.tasks, range(args.rep_start, args.rep_start + args.reps), args.arms, args.seed)
    print(f"planned runs: {result['planned_runs']} "
          + ", ".join(f"{arm} {count}" for arm, count in result["per_arm"].items()))
    for arm, task, rep in result["schedule"]:
        print(f"{arm} {task} r{rep}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
