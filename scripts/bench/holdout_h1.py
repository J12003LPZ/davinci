"""Pre-registered schedule, privacy safeguards and evaluator for the independent holdout study H1.

Prepares and evaluates only. It never launches a model run: `commands` prints the PowerShell lines an
approved operator would run, and `report` reads result rows that already exist.

Three arms, six independently authored Python/pytest tasks, three repetitions (54 runs at most):
  A  DaVinci, PR #193 binary, transport "auto" (HTTP)
  B  the exact same binary and settings, transport "public-websocket"
  C  pinned Codex CLI, same model and effort
A and B differ only in `transport`. There are no warm-ups: every run is measured.

Unit of launch: the private-suite loader runs a complete split per campaign, so one campaign is one
(arm, repetition) sweep of all six tasks. A stage is one repetition (three campaigns, 18 runs) and
needs one fresh sign-in. Sweep order rotates over repetitions so each arm opens, middles and closes a stage once.

Conclusions are limited to multi-file Python repositories tested with pytest, six tasks, one model. The
two-of-eighteen success margin is an operational tolerance, not a statistically established non-inferiority.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import statistics
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parent))

ARMS = ("A", "B", "C")
ARM_HARNESS = {"A": "davinci", "B": "davinci", "C": "codex"}
ARM_TRANSPORT = {"A": "auto", "B": "public-websocket"}
DAVINCI_ARMS = ("A", "B")
REPS = 3
TASK_COUNT = 6
STAGES = REPS
BLOCKS = TASK_COUNT * REPS
MAX_RUNS = BLOCKS * len(ARMS)
PREFIX = "h1"
# Rotation over repetitions: each arm holds each position once. Arms cannot interleave inside a rep
# because a private split is run whole.
REP_ORDERS = (("A", "B", "C"), ("B", "C", "A"), ("C", "A", "B"))

# Registered constants. The binary hashes and the suite digest live in the pins file, which the
# preregistration records by hash before any run.
SOURCE_SHA = "1628438c4ebfcb1e23aee6c381533da48362eec1"
DAVINCI_SHA256 = "364b229275f1a7114ff36fa074bd53c9c7da3b22992b6567af921faef7e3a55b"
CODEX_VERSION = "codex-cli 0.162.1"
CODEX_SHA256 = "dd13bdb162a184fa04039488b234a8258ef9b76f84e926c505d5742b19249a6a"
CODEX_HOST_SHA256 = "9f865a748c984492ca9b69aac75680e4954f31311c43ec33ddbdae0ed77bd4bd"
MODEL = "gpt-6-luna"
EFFORT = "high"
SERVICE_TIER = "default"
SPLIT = "holdout"
LANGUAGE = "python"

# Decision rule, fixed before any run.
SUCCESS_TOLERANCE = 2          # of 18: operational tolerance, not a statistical non-inferiority margin
MAX_TASK_PASS_DEFICIT = 1      # no task may lose more than one repetition against the comparator
MAX_TOKENS_PER_SUCCESS_RATIO = 0.90   # (uncached input + output) per verified success, DaVinci / Codex
MAX_MEDIAN_TASK_WALL_RATIO = 1.00
MIN_TASKS_FASTER = TASK_COUNT // 2
SAFETY_STOP_LAG = 2            # cumulative passes B is behind A over paired blocks
MAX_UNKNOWN_ATTEMPTS_RUN = 1
MAX_UNKNOWN_ATTEMPTS_TOTAL = 3
LEAK_MIN_LINE = 60             # hidden-test lines shorter than this are too generic to be a needle
TASK_ID = re.compile(r"[a-z0-9][a-z0-9_-]{1,40}")
HEX40, HEX64 = re.compile(r"[0-9a-f]{40}"), re.compile(r"[0-9a-f]{64}")


# ---------------------------------------------------------------- schedule

def schedule():
    """The nine campaigns in launch order. Each runs all six tasks of one arm for one repetition."""
    units = []
    for rep in range(REPS):
        for position, arm in enumerate(REP_ORDERS[rep]):
            units.append({"arm": arm, "rep": rep, "stage": rep + 1, "position": position})
    for seq, unit in enumerate(units, 1):
        unit["seq"] = seq
        unit["name"] = unit_name(unit)
    return units


def unit_name(unit):
    return "%s-%02d-%s-r%d" % (PREFIX, unit["seq"], unit["arm"], unit["rep"])


def settings_for(arm, base):
    out = json.loads(json.dumps(base))
    out["transport"] = ARM_TRANSPORT[arm]
    return out


def settings_errors(settings_dir, base):
    """Both DaVinci arm files must exist and differ from the base only in `transport`."""
    errors = []
    for arm in DAVINCI_ARMS:
        try:
            found = json.loads((Path(settings_dir) / ("%s.json" % arm)).read_text(encoding="utf-8"))
        except (OSError, ValueError):
            errors.append("%s: settings file missing or unreadable" % arm)
            continue
        if found != settings_for(arm, base):
            errors.append("%s: settings differ from the base beyond transport" % arm)
    return errors


def ps(value):
    """A PowerShell single-quoted literal: no expansion, no word splitting."""
    return "'" + str(value).replace("'", "''") + "'"


def commands(root, settings_dir, suite, davinci, codex, stage=None, bench="scripts/bench/bench.py"):
    lines = []
    for unit in schedule():
        if stage is not None and unit["stage"] != stage:
            continue
        env = "$env:BENCH_RUNS = %s; " % ps("%s\\%s" % (root, unit["name"]))
        if unit["arm"] == "C":
            env += "$env:BENCH_CODEX = %s; Remove-Item Env:BENCH_DAVINCI -ErrorAction SilentlyContinue; " % ps(codex)
            extra = ""
        else:
            env += "$env:BENCH_DAVINCI = %s; Remove-Item Env:BENCH_CODEX -ErrorAction SilentlyContinue; " % ps(davinci)
            extra = " --settings %s" % ps("%s\\%s.json" % (settings_dir, unit["arm"]))
        lines.append(
            "%spython %s run --harness %s --private-suite %s --split %s --tasks all --reps 1 --rep-start %d "
            "--variant %s%s --grading-timeout 120"
            % (env, ps(bench), ARM_HARNESS[unit["arm"]], ps(suite), SPLIT, unit["rep"], unit["name"], extra))
    return lines


# ---------------------------------------------------------------- pins and locations

def file_sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def pins_errors(pins):
    """Shape and registered-constant checks on the pins file. Missing is an error, never a pass."""
    if not isinstance(pins, dict):
        return ["pins file must be an object"]
    errors = []
    registered = {"source_sha": SOURCE_SHA, "davinci_sha256": DAVINCI_SHA256, "codex_sha256": CODEX_SHA256,
                  "codex_host_sha256": CODEX_HOST_SHA256, "codex_version": CODEX_VERSION, "model": MODEL,
                  "effort": EFFORT, "service_tier": SERVICE_TIER}
    for field, pin in registered.items():
        if pins.get(field) != pin:
            errors.append("pins.%s is not the registered value" % field)
    if not isinstance(pins.get("private_suite_digest"), str) or not HEX64.fullmatch(pins["private_suite_digest"]):
        errors.append("pins.private_suite_digest must be a sha256 hex digest")
    ids = pins.get("task_ids")
    if (not isinstance(ids, list) or len(ids) != TASK_COUNT or len(set(ids)) != TASK_COUNT
            or any(not isinstance(i, str) or not TASK_ID.fullmatch(i) for i in ids)):
        errors.append("pins.task_ids must be %d unique opaque ids" % TASK_COUNT)
    if not isinstance(pins.get("harness_sha"), str) or not HEX40.fullmatch(pins["harness_sha"]):
        errors.append("pins.harness_sha must be a full commit")
    return errors


def pin_errors(pins, davinci=None, codex=None, codex_host=None, harness_sha=None):
    """Observed identities against the pins. Anything not supplied is an error."""
    errors = pins_errors(pins)
    for label, path, key in (("davinci binary", davinci, "davinci_sha256"), ("codex binary", codex, "codex_sha256"),
                             ("codex host", codex_host, "codex_host_sha256")):
        try:
            observed = file_sha256(path)
        except (OSError, TypeError):
            errors.append("%s: file unreadable" % label)
            continue
        if isinstance(pins, dict) and observed != pins.get(key):
            errors.append("%s hash %s is not the pinned %s" % (label, observed[:12], str(pins.get(key))[:12]))
    if not harness_sha or not isinstance(pins, dict) or harness_sha != pins.get("harness_sha"):
        errors.append("harness commit %s is not the frozen %s" % (
            harness_sha, pins.get("harness_sha") if isinstance(pins, dict) else None))
    return errors


def _inside(child, parent):
    try:
        Path(child).resolve().relative_to(Path(parent).resolve())
        return True
    except ValueError:
        return False


def location_errors(suite, forbidden):
    """The private suite must not sit where an evaluated agent can read it, or where results are written.

    `forbidden` maps a label to a directory. The suite may not be inside one, nor contain one.
    """
    errors = []
    suite = Path(suite)
    if not suite.is_dir():
        return ["suite directory does not exist"]
    for label, directory in sorted(forbidden.items()):
        if _inside(suite, directory) or _inside(directory, suite):
            errors.append("suite overlaps %s" % label)
    return errors


def default_forbidden(repo, results_root, home=None):
    home = Path(home) if home else Path.home()
    paths = {"the harness checkout": repo, "the results root": results_root,
             "the system temp directory": tempfile.gettempdir(), "the current directory": Path.cwd()}
    for name in (".pi", ".codex", ".claude", ".davinci"):
        paths["agent config " + name] = home / name
    return {label: path for label, path in paths.items() if path}


def suite_errors(frozen):
    """Shape of the frozen suite.json. Reads only labels and ids; never task content."""
    if not isinstance(frozen, dict):
        return ["suite.json is not an object"]
    errors, tasks, labels = [], frozen.get("tasks"), frozen.get("labels")
    if not isinstance(tasks, list) or len(tasks) != TASK_COUNT or len(set(tasks)) != TASK_COUNT:
        return ["the suite must hold exactly %d distinct tasks" % TASK_COUNT]
    if not isinstance(labels, dict):
        return ["suite labels missing"]
    repositories = set()
    for tid in tasks:
        label = labels.get(tid)
        if not TASK_ID.fullmatch(str(tid)):
            errors.append("task id %r is not an opaque id" % str(tid)[:20])
        if not isinstance(label, dict):
            errors.append("%s: labels missing" % tid)
            continue
        if label.get("split") != SPLIT:
            errors.append("%s: split is not %s" % (tid, SPLIT))
        if str(label.get("language", "")).lower() != LANGUAGE:
            errors.append("%s: language is not %s" % (tid, LANGUAGE))
        if label.get("size_class") != "large":
            errors.append("%s: size class is not large" % tid)
        repositories.add(label.get("repository_id"))
    if len(repositories) != TASK_COUNT:
        errors.append("each task needs its own repository id")
    return errors


def make_pins(suite, harness_sha):
    """Pins file for a frozen suite: registered constants, the suite digest and the opaque task ids."""
    import private_suite
    try:
        frozen, tasks, digest = private_suite.load_frozen(suite, SPLIT)
    except (OSError, ValueError) as error:
        return None, ["suite refused by the loader: %s" % error]
    errors = suite_errors(frozen)
    pins = {"source_sha": SOURCE_SHA, "davinci_sha256": DAVINCI_SHA256, "codex_sha256": CODEX_SHA256,
            "codex_host_sha256": CODEX_HOST_SHA256, "codex_version": CODEX_VERSION, "model": MODEL,
            "effort": EFFORT, "service_tier": SERVICE_TIER, "private_suite_digest": digest,
            "task_ids": sorted(tasks), "harness_sha": harness_sha}
    return pins, errors + pins_errors(pins)


# ---------------------------------------------------------------- leak scan

def _lines(path):
    try:
        return [line.strip() for line in Path(path).read_text(encoding="utf-8", errors="replace").splitlines()]
    except OSError:
        return []


def _path_variants(suite):
    posix = suite.as_posix()
    variants = {str(suite), posix}
    if suite.drive:  # Git Bash and MSYS spell D:/x as /d/x
        variants.add("/" + suite.drive[0] + posix[2:])
    return variants


def needles(suite):
    """Search strings that would prove an agent saw suite content: the suite path, distinctive hidden-test lines
    and distinctive reference-solution lines.

    Returns {digest12: needle}. Only the digests ever leave this module.
    """
    suite = Path(suite).resolve()
    found = {text: "path" for text in _path_variants(suite)}
    for task_dir in sorted(p for p in suite.iterdir() if p.is_dir()):
        known = set()
        for file in (task_dir / "repo").rglob("*"):
            if file.is_file():
                known.update(_lines(file))
        spec = task_dir / "task.json"
        prompt = ""
        if spec.is_file():
            try:
                prompt = json.loads(spec.read_text(encoding="utf-8")).get("prompt", "")
            except ValueError:
                prompt = ""
        for tree in ("hidden", "solution"):
            for file in (task_dir / tree).rglob("*") if (task_dir / tree).is_dir() else []:
                if file.is_file():
                    for line in _lines(file):
                        if len(line) >= LEAK_MIN_LINE and line not in known and line not in prompt:
                            found[line] = tree
    return {hashlib.sha256(text.encode("utf-8")).hexdigest()[:12]: text for text in found}


def transcript_files(root):
    """Agent-written text only. Never auth.json or binaries."""
    out = []
    for pattern in ("*.stdout.jsonl", "*.stderr.txt", "*.otel.json"):
        out += [p for p in Path(root).rglob(pattern) if "_agent" not in p.parts]
    return sorted(out)


def _strings(value, out):
    if isinstance(value, str):
        out.append(value)
    elif isinstance(value, dict):
        for key, item in value.items():
            _strings(key, out)
            _strings(item, out)
    elif isinstance(value, list):
        for item in value:
            _strings(item, out)


def haystacks(text):
    """The file as written, and the strings a JSON reader would decode from it, lowercased.

    JSON escapes quotes, tabs, backslashes and non-ASCII characters, so a copied line reads differently on disk.
    """
    decoded = []
    documents = [text] + text.splitlines()
    for document in documents:
        try:
            _strings(json.loads(document), decoded)
        except ValueError:
            continue
    raw = text.lower()
    return [raw, raw.replace("\\\\", "\\"), "\n".join(decoded).lower()]


def leak_scan(root, suite, expected_runs=None, expected_digest=None):
    """Counts only. The report never carries a file name from the suite or any matched text.

    CLEAN needs the right suite, a readable results root, and a transcript for every run that is expected.
    """
    result = {"status": "NOT_SCANNED", "files_scanned": 0, "needles": 0, "campaigns_with_hits": {}}
    if expected_digest is not None:
        import private_suite
        try:
            digest = private_suite.load_frozen(suite, SPLIT)[2]
        except (OSError, ValueError):
            return result
        if digest != expected_digest:
            return dict(result, status="WRONG_SUITE")
    try:
        table = needles(suite)
    except OSError:
        return result
    files = transcript_files(root)
    hits = {}
    for path in files:
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        pools = haystacks(text)
        for key, needle in table.items():
            low = needle.lower()
            escaped = json.dumps(needle)[1:-1].lower()
            if any(low in pool or low.replace("\\", "/") in pool.replace("\\", "/") for pool in pools) or escaped in pools[0]:
                hits.setdefault(path.relative_to(root).parts[0], set()).add(key)
    stdout_files = sum(1 for p in files if p.name.endswith(".stdout.jsonl"))
    result.update(files_scanned=len(files), needles=len(table),
                  campaigns_with_hits={name: len(keys) for name, keys in sorted(hits.items())})
    if hits:
        result["status"] = "LEAK"
    elif not files or (expected_runs is not None and stdout_files < expected_runs):
        result["status"] = "INCOMPLETE_SCAN" if files else "NOT_SCANNED"
    else:
        result["status"] = "CLEAN"
    return result


# ---------------------------------------------------------------- rows

def load_rows(root):
    """{(arm, task, rep): row}, plus structural errors. The arm comes from the campaign directory."""
    rows, errors = {}, []
    by_name = {u["name"]: u for u in schedule()}
    for path in sorted(Path(root).glob(PREFIX + "-*/results.jsonl")):
        unit = by_name.get(path.parent.name)
        if unit is None:
            errors.append("campaign directory %s is outside the schedule" % path.parent.name)
            continue
        try:
            lines = path.read_text(encoding="utf-8").splitlines()
        except (OSError, ValueError):
            errors.append("results of %s are unreadable" % unit["name"])
            continue
        for line in lines:
            if not line.strip():
                continue
            try:
                row = json.loads(line)
            except ValueError:
                errors.append("unreadable row in %s" % unit["name"])
                continue
            if not isinstance(row, dict) or not isinstance(row.get("task"), str) or type(row.get("rep")) is not int:
                errors.append("row with an invalid task or repetition in %s" % unit["name"])
                continue
            key = (unit["arm"], row["task"], row["rep"])
            if key in rows:
                errors.append("duplicate row %s" % (key,))
            row["_unit"] = unit
            rows[key] = row
    return rows, errors


def _count(value):
    return type(value) is int and value >= 0


def usage_known(row):
    totals = (row.get("input_tokens"), row.get("cached_tokens"), row.get("output_tokens"))
    if not all(_count(v) for v in totals) or row["cached_tokens"] > row["input_tokens"]:
        return False
    if row.get("harness") == "codex":
        return row.get("request_metrics_complete") is True
    return type(row.get("usage_unknown_attempts")) is int and row["usage_unknown_attempts"] == 0


def scorable(row):
    """Everything `metric` reads must be present and well formed, or the run is unscorable (never zero)."""
    wall = row.get("wall_s")
    return (usage_known(row) and type(row.get("pass")) is bool and _count(row.get("requests"))
            and isinstance(wall, (int, float)) and type(wall) is not bool and math.isfinite(wall) and wall > 0
            and isinstance(row.get("unrelated"), list))


def identity_errors(rows, pins):
    errors, fixtures, base_settings = [], set(), None
    for (arm, task, rep), row in sorted(rows.items(), key=lambda kv: (kv[1]["_unit"]["seq"], str(kv[0][1]))):
        tag = "%s %s r%s" % (arm, task, rep)
        unit = row["_unit"]
        if task not in pins["task_ids"]:
            errors.append("%s: task is not one of the pinned ids" % tag)
        if rep != unit["rep"]:
            errors.append("%s: repetition does not match its campaign" % tag)
        if row.get("harness") != ARM_HARNESS[arm]:
            errors.append("%s: harness %r" % (tag, row.get("harness")))
        for field, pin in (("model", MODEL), ("effort_policy", EFFORT), ("service_tier", SERVICE_TIER),
                           ("private_suite_digest", pins["private_suite_digest"]), ("split", SPLIT),
                           ("language", LANGUAGE), ("task_set", "private"), ("variant", unit["name"])):
            if row.get(field) is None:
                errors.append("%s: %s not recorded" % (tag, field))
            elif row[field] != pin:
                errors.append("%s: %s is not the pinned value" % (tag, field))
        binary = pins["codex_sha256"] if arm == "C" else pins["davinci_sha256"]
        if row.get("binary_sha256") != binary:
            errors.append("%s: binary is not the pinned one" % tag)
        if arm == "C":
            if row.get("version") != pins["codex_version"]:
                errors.append("%s: codex version is not the pinned one" % tag)
            settings = row.get("effective_settings")
            if (not isinstance(settings, dict) or settings.get("ignore_user_config") is not True
                    or settings.get("effort") != EFFORT or settings.get("service_tier") != SERVICE_TIER):
                errors.append("%s: codex settings are not the pinned ones" % tag)
        else:
            if row.get("source_sha") != pins["source_sha"]:
                errors.append("%s: source is not the pinned one" % tag)
            settings = row.get("effective_settings")
            if not isinstance(settings, dict):
                errors.append("%s: effective settings not recorded" % tag)
            else:
                if settings.get("transport") != ARM_TRANSPORT[arm]:
                    errors.append("%s: arm ran with transport %r" % (tag, settings.get("transport")))
                rest = {k: v for k, v in settings.items() if k != "transport"}
                if base_settings is None:
                    base_settings = rest
                elif rest != base_settings:
                    errors.append("%s: settings differ between A and B beyond transport" % tag)
            labels = row.get("provider_transports")
            if not isinstance(labels, dict) or not labels or not all(_count(v) for v in labels.values()):
                errors.append("%s: provider transports not recorded as counts" % tag)
            elif arm == "A" and (set(labels) != {"http"} or labels["http"] < 1):
                errors.append("%s: arm A used %s" % (tag, sorted(labels)))
            elif arm == "B" and (not set(labels) <= {"websocket", "http"} or labels.get("websocket", 0) < 1):
                errors.append("%s: arm B used %s" % (tag, sorted(labels)))
            elif _count(row.get("provider_attempts")) and sum(labels.values()) != row["provider_attempts"]:
                errors.append("%s: transport counts do not add up to the provider attempts" % tag)
        if type(row.get("pass")) is not bool:
            errors.append("%s: pass is not a boolean" % tag)
        elif row["pass"] and (row.get("exit") != 0 or row.get("unrelated") or row.get("cleanup_complete") is False):
            errors.append("%s: pass is inconsistent with its exit code, unrelated edits or cleanup" % tag)
        if row.get("reasoning_included_in_output") is not True:
            errors.append("%s: reasoning tokens are not recorded as part of output" % tag)
        if row.get("fixture_hash") is None:
            errors.append("%s: fixture hash not recorded" % tag)
        fixtures.add(json.dumps(row.get("fixture_hash"), sort_keys=True, default=str))
    if len(fixtures) > 1:
        errors.append("the suite fixture changed between campaigns")
    return errors


def ordered(rows):
    return sorted(rows.items(), key=lambda kv: (kv[1]["_unit"]["seq"], str(kv[1].get("started_at"))))


def paired_passes(rows, left, right):
    """Verified passes of two arms over the (task, rep) blocks where both have a row."""
    keys = sorted({(t, r) for (a, t, r) in rows if a == left} & {(t, r) for (a, t, r) in rows if a == right},
                  key=str)
    return len(keys), sum(rows[(left, t, r)].get("pass") is True for t, r in keys), sum(
        rows[(right, t, r)].get("pass") is True for t, r in keys)


def run_stop(key, row, unknown_so_far, earlier):
    """Predefined stop for one finished run; None means continue. A stop ends the whole study."""
    arm, task, rep = key
    tag = "%s %s r%s" % (arm, task, rep)
    if row.get("stop_reason") is not None:
        return "%s: harness stop: %s" % (tag, str(row["stop_reason"])[:60])
    unknown = row.get("usage_unknown_attempts")
    if arm in DAVINCI_ARMS:
        if not _count(unknown):
            return "%s: attempts without usage not recorded as a count" % tag
        if unknown > MAX_UNKNOWN_ATTEMPTS_RUN:
            return "%s: %d attempts without usage" % (tag, unknown)
        if unknown_so_far + unknown > MAX_UNKNOWN_ATTEMPTS_TOTAL:
            return "%s: %d attempts without usage in total" % (tag, unknown_so_far + unknown)
        if row.get("pass") is not True and unknown > 0:
            return "%s: failed verification with %d attempts without usage (cause not separable)" % (tag, unknown)
    if arm == "B" and (row.get("input_tokens") is None or unknown > 0):
        return "%s: usage incomplete on a WebSocket run" % tag
    if arm == "C" and row.get("request_metrics_complete") is not True:
        return "%s: codex request telemetry incomplete" % tag
    if row.get("output_tokens") is None:
        return "%s: output tokens not recorded" % tag
    seen = dict(earlier)
    seen[key] = row
    blocks, a_passes, b_passes = paired_passes(seen, "A", "B")
    if a_passes - b_passes >= SAFETY_STOP_LAG:
        return "safety: B is %d verified passes behind A over %d paired blocks" % (a_passes - b_passes, blocks)
    return None


def first_stop(rows, pins):
    """Replay the stop rules in launch order. Identity problems stop the study before anything else."""
    identity = identity_errors(rows, pins)
    if identity:
        return identity[0]
    unknown_total, earlier = 0, {}
    for key, row in ordered(rows):
        reason = run_stop(key, row, unknown_total, earlier)
        if reason:
            return reason
        earlier[key] = row
        if key[0] in DAVINCI_ARMS:
            unknown_total += row["usage_unknown_attempts"]
    return None


# ---------------------------------------------------------------- fallback evaluation

def transcript_path(root, row):
    unit = row["_unit"]
    return Path(root) / unit["name"] / row["harness"] / ("%s-r%s.stdout.jsonl" % (row["task"], row["rep"]))


def tool_call_integrity(path):
    """Duplicated, unfinished or orphaned tool calls in a transcript. None when it cannot be verified.

    A transcript without a single tool event proves nothing, so it is also None.
    """
    try:
        text = Path(path).read_text(encoding="utf-8")
    except (OSError, ValueError):
        return None
    starts, ends = {}, {}
    for line in text.splitlines():
        try:
            event = json.loads(line)
        except ValueError:
            continue
        if not isinstance(event, dict) or not isinstance(event.get("toolCallId"), str):
            continue
        kind, call = event.get("type"), event["toolCallId"]
        if kind == "tool_execution_start":
            starts[call] = starts.get(call, 0) + 1
        elif kind == "tool_execution_end":
            ends[call] = ends.get(call, 0) + 1
    if not starts and not ends:
        return None
    return {"starts": sum(starts.values()),
            "duplicated": sum(1 for n in starts.values() if n > 1) + sum(1 for n in ends.values() if n > 1),
            "unfinished": sum(1 for call in starts if call not in ends),
            "orphaned_ends": sum(1 for call in ends if call not in starts)}


def fallback_events(rows, root=None):
    """One entry per arm B run that sent any request over HTTP. No content, counts and a classification."""
    events = []
    for (arm, task, rep), row in sorted(rows.items(), key=lambda kv: (kv[0][2], str(kv[0][1]))):
        labels = row.get("provider_transports") or {}
        if arm != "B" or not labels.get("http"):
            continue
        integrity = tool_call_integrity(transcript_path(root, row)) if root is not None else None
        leaf, calls = row.get("executed_leaf_operations"), row.get("tool_calls")
        counts_agree = _count(leaf) and _count(calls) and leaf == calls
        if integrity is None or not (_count(leaf) and _count(calls)) or integrity["starts"] != calls:
            kind = "UNVERIFIED"
        elif (integrity["duplicated"] or integrity["orphaned_ends"] or integrity["unfinished"]
              or not counts_agree):
            kind = "LOST_OR_DUPLICATED"
        else:
            kind = "RECOVERABLE"
        events.append({"task": task, "rep": rep, "http_requests": labels["http"],
                       "websocket_requests": labels.get("websocket", 0), "passed": row.get("pass") is True,
                       "provider_attempts": row.get("provider_attempts") if _count(row.get("provider_attempts")) else None,
                       "logical_requests": row.get("logical_requests") if _count(row.get("logical_requests")) else None,
                       "classification": kind, "tool_call_integrity": integrity,
                       "operation_counts_agree": counts_agree})
    return events


def fallback_verdict(events, b_runs):
    if not events:
        return {"verdict": "NO_FALLBACK_OBSERVED", "event_count": 0, "b_runs": b_runs,
                "rule_of_three_upper_95": round(3 / b_runs, 3) if b_runs else None,
                "note": "Zero events in %d runs bounds the true per-run rate below the stated value with about 95%% "
                        "confidence. It does not show the rate is acceptable for production." % b_runs}
    if any(e["classification"] == "LOST_OR_DUPLICATED" or not e["passed"] for e in events):
        verdict = "NOT_ACCEPTABLE"
    elif any(e["classification"] == "UNVERIFIED" for e in events):
        verdict = "UNVERIFIED"
    else:
        verdict = "ACCEPTABLE_PENDING_REVIEW"
    return {"verdict": verdict, "event_count": len(events), "b_runs": b_runs,
            "rate": round(len(events) / b_runs, 3) if b_runs else None,
            "note": "A human reviews every event. The rate alone is not the criterion."}


# ---------------------------------------------------------------- analysis

def metric(row):
    return {"pass": row["pass"], "wall": row["wall_s"], "requests": row["requests"],
            "uncached": row["input_tokens"] - row["cached_tokens"], "output": row["output_tokens"],
            "unrelated": bool(row["unrelated"])}


def _tokens(ms):
    return sum(m["uncached"] + m["output"] for m in ms)


def compare(measured, arm, base):
    """DaVinci arm against a base arm over the blocks where both finished. No thresholds applied here."""
    keys = sorted({(t, r) for (a, t, r) in measured if a == arm} & {(t, r) for (a, t, r) in measured if a == base},
                  key=str)
    tasks = sorted({t for t, _ in keys}, key=str)
    mine = [measured[(arm, t, r)] for t, r in keys]
    theirs = [measured[(base, t, r)] for t, r in keys]
    per_task = []
    for task in tasks:
        a = [measured[(arm, t, r)] for t, r in keys if t == task]
        b = [measured[(base, t, r)] for t, r in keys if t == task]
        per_task.append({"task": task, "reps": len(a), "passes": [sum(m["pass"] for m in b), sum(m["pass"] for m in a)],
                         "median_wall_ratio": statistics.median(m["wall"] for m in a) / statistics.median(
                             m["wall"] for m in b)})
    per_rep = []
    for rep in sorted({r for _, r in keys}):
        a = [measured[(arm, t, r)] for t, r in keys if r == rep]
        b = [measured[(base, t, r)] for t, r in keys if r == rep]
        per_rep.append({"rep": rep, "passes": [sum(m["pass"] for m in b), sum(m["pass"] for m in a)]})
    passes = [sum(m["pass"] for m in theirs), sum(m["pass"] for m in mine)]
    per_success = lambda ms, wins: _tokens(ms) / wins if wins else None
    ratio = lambda x, y: x / y if x is not None and y else None
    return {"blocks": len(keys), "passes": passes,
            "worst_task_pass_deficit": max((t["passes"][0] - t["passes"][1] for t in per_task), default=0),
            "tokens_per_success": [per_success(theirs, passes[0]), per_success(mine, passes[1])],
            "tokens_per_success_ratio": ratio(per_success(mine, passes[1]), per_success(theirs, passes[0])),
            "wall_total_ratio": ratio(sum(m["wall"] for m in mine), sum(m["wall"] for m in theirs)),
            "request_ratio": ratio(sum(m["requests"] for m in mine), sum(m["requests"] for m in theirs)),
            "tasks_faster": sum(t["median_wall_ratio"] < 1.0 for t in per_task),
            "median_task_wall_ratio": statistics.median(t["median_wall_ratio"] for t in per_task) if per_task else None,
            "unrelated_edit_runs": [sum(m["unrelated"] for m in theirs), sum(m["unrelated"] for m in mine)],
            "per_task": per_task, "per_rep": per_rep}


def success_label(c):
    guard = c["worst_task_pass_deficit"] <= MAX_TASK_PASS_DEFICIT
    base, mine = c["passes"]
    if base == 0 and mine == 0:
        return "NO_SIGNAL"
    if mine > base and guard:
        return "AHEAD"
    if mine >= base - SUCCESS_TOLERANCE and guard:
        return "MATCHES"
    return "BEHIND"


def efficiency(c):
    ratio = c["tokens_per_success_ratio"]
    checks = {"tokens_per_success": ratio is not None and ratio <= MAX_TOKENS_PER_SUCCESS_RATIO,
              "median_task_wall": c["median_task_wall_ratio"] is not None
              and c["median_task_wall_ratio"] <= MAX_MEDIAN_TASK_WALL_RATIO,
              "tasks_faster": c["tasks_faster"] >= MIN_TASKS_FASTER}
    return {"checks": checks, "all": all(checks.values())}


def vs_codex(measured, arm):
    c = compare(measured, arm, "C")
    label = success_label(c)
    out = {"success": label, "comparison": c}
    if label in ("BEHIND", "NO_SIGNAL"):
        out["efficiency"] = "NOT_SCORED"
        out["outcome"] = label
    else:
        out["efficiency"] = efficiency(c)
        if c["tokens_per_success_ratio"] is None:
            out["outcome"] = "%s_EFFICIENCY_UNDEFINED" % label
        else:
            out["outcome"] = ("%s_AND_EFFICIENT" % label) if out["efficiency"]["all"] else "%s_NOT_EFFICIENT" % label
    return out


NOTE = ("Six independently authored Python/pytest tasks, one model, three repetitions each. Repetitions of a task are "
        "not independent samples. The success margin is an operational tolerance, not a statistically established "
        "non-inferiority. No p-value is reported or used. A result supports a claim about this suite only, never "
        "superiority and never other languages or task shapes.")


def default_gate(passes_ok, fallback):
    """Evidence for the WebSocket default decision. A human still decides."""
    if not passes_ok or fallback == "NOT_ACCEPTABLE":
        return "FAIL"
    return "UNVERIFIED" if fallback == "UNVERIFIED" else "PASS"


def evaluate(rows, pins, leak=None, root=None, load_errors=()):
    errors = pins_errors(pins) + list(load_errors)
    if errors:
        return {"verdict": "INVALID", "errors": errors}
    stop = first_stop(rows, pins)
    if leak is not None and leak.get("status") != "CLEAN":
        return {"verdict": "CONTAMINATION_REVIEW", "leak_scan": leak, "stop": stop,
                "note": "the scan found suite content or could not vouch for the transcripts; a human reviews this "
                        "before any use of the result"}
    if stop:
        return {"verdict": "STOPPED", "stop": stop, "runs_present": len(rows),
                "note": "a stop ends the study; no reruns, no stitching"}
    if leak is None:
        return {"verdict": "NEEDS_LEAK_SCAN", "runs_present": len(rows),
                "note": "results stay unread until the privacy scan has run"}
    measured = {key: metric(row) for key, row in rows.items() if scorable(row)}
    unscorable = sorted("%s %s r%s" % (a, t, r) for (a, t, r) in rows if (a, t, r) not in measured)
    expected = {(arm, task, rep) for arm in ARMS for task in pins["task_ids"] for rep in range(REPS)}
    missing = sorted("%s %s r%s" % k for k in expected - set(rows))
    if unscorable or missing:
        return {"verdict": "INCOMPLETE", "runs_present": len(rows), "of": MAX_RUNS, "missing_runs": missing,
                "unscorable_runs": unscorable, "leak_scan": leak, "note": "No verdict. Unknown usage stays unknown and "
                "no block was dropped or imputed."}
    b_runs = sum(1 for key in rows if key[0] == "B")
    events = fallback_events(rows, root)
    fallback = {"events": events, **fallback_verdict(events, b_runs)}
    b_vs_a = compare(measured, "B", "A")
    return {"verdict": "COMPLETE", "leak_scan": leak, "runs": len(rows),
            "passes": {arm: sum(m["pass"] for (a, _, _), m in measured.items() if a == arm) for arm in ARMS},
            "b_vs_codex": vs_codex(measured, "B"), "a_vs_codex": vs_codex(measured, "A"),
            "b_vs_a": {"comparison": b_vs_a, "passes_gate": b_vs_a["passes"][1] >= b_vs_a["passes"][0],
                       "gate": default_gate(b_vs_a["passes"][1] >= b_vs_a["passes"][0], fallback["verdict"])},
            "fallback": fallback, "decision": "HUMAN_REVIEW_REQUIRED", "note": NOTE}


# ---------------------------------------------------------------- cli

def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = ap.add_subparsers(dest="mode", required=True)
    sub.add_parser("schedule")
    c = sub.add_parser("commands")
    for name in ("root", "settings-dir", "suite", "davinci", "codex"):
        c.add_argument("--" + name, required=True)
    c.add_argument("--stage", type=int, choices=range(1, STAGES + 1))
    r = sub.add_parser("report")
    r.add_argument("--root", required=True)
    r.add_argument("--pins", required=True)
    r.add_argument("--suite", help="frozen private suite directory, for the leak scan")
    p = sub.add_parser("verify-pins")
    for name in ("pins", "davinci", "codex", "codex-host", "harness-sha"):
        p.add_argument("--" + name, required=True)
    loc = sub.add_parser("check-location")
    loc.add_argument("--suite", required=True)
    loc.add_argument("--results-root", required=True)
    loc.add_argument("--repo", required=True)
    m = sub.add_parser("make-pins", help="print the pins file for a frozen suite; writes nothing")
    m.add_argument("--suite", required=True)
    m.add_argument("--harness-sha", required=True)
    s = sub.add_parser("verify-settings")
    s.add_argument("--settings-dir", required=True)
    args = ap.parse_args(argv)
    if args.mode == "schedule":
        for unit in schedule():
            print("%d stage %d position %d arm %s rep %d  %s" % (unit["seq"], unit["stage"], unit["position"] + 1,
                                                                 unit["arm"], unit["rep"], unit["name"]))
    elif args.mode == "commands":
        print("\n".join(commands(args.root, args.settings_dir, args.suite, args.davinci, args.codex, args.stage)))
    elif args.mode == "verify-pins":
        pins = json.loads(Path(args.pins).read_text(encoding="utf-8"))
        errors = pin_errors(pins, args.davinci, args.codex, args.codex_host, args.harness_sha)
        print(json.dumps({"ok": not errors, "errors": errors}, indent=2))
        sys.exit(1 if errors else 0)
    elif args.mode == "make-pins":
        pins, errors = make_pins(args.suite, args.harness_sha)
        print(json.dumps({"ok": not errors, "errors": errors, "pins": pins}, indent=2))
        sys.exit(1 if errors else 0)
    elif args.mode == "check-location":
        errors = location_errors(args.suite, default_forbidden(args.repo, args.results_root))
        print(json.dumps({"ok": not errors, "errors": errors}, indent=2))
        sys.exit(1 if errors else 0)
    elif args.mode == "verify-settings":
        import runner
        errors = settings_errors(args.settings_dir, runner.BASE_SETTINGS)
        print(json.dumps({"ok": not errors, "errors": errors}, indent=2))
        sys.exit(1 if errors else 0)
    else:
        pins = json.loads(Path(args.pins).read_text(encoding="utf-8"))
        rows, load_errors = load_rows(args.root)
        leak = (leak_scan(args.root, args.suite, expected_runs=len(rows), expected_digest=pins.get("private_suite_digest"))
                if args.suite and isinstance(pins, dict) else None)
        print(json.dumps(evaluate(rows, pins, leak, args.root, load_errors), indent=2, default=str))


if __name__ == "__main__":
    main()
