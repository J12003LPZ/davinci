"""Campaign setup, process-lifetime ownership, and measurement boundaries."""
import json
import importlib.util
from pathlib import Path
import shutil
import os
import signal
import subprocess
import time
import hashlib
import platform
import re
import sys
import uuid
import tempfile
from contextlib import contextmanager
from datetime import datetime, timezone

from campaign import LEGACY_TASKS, digest, file_hash

_release_spec = importlib.util.spec_from_file_location(
    "davinci_release_identity", Path(__file__).resolve().parents[1] / "release_identity.py")
release_identity = importlib.util.module_from_spec(_release_spec)
_release_spec.loader.exec_module(release_identity)

CLEAN_DIFF_HASH = hashlib.sha256(b"").hexdigest()
_campaign_owner = None

BASE_SETTINGS = {"decisionIntelligence": {"enabled": False},
    "compaction": {"enabled": True, "reserveTokens": 16384, "keepRecentTokens": 20000},
    "showCacheMissNotices": False, "transport": "auto", "serviceTier": None,
    "effortPolicy": "fixed", "toolSurface": "full", "autoVerify": True,
    "promptProfile": "stable"}

GRADING_ISOLATIONS = ("diagnostic-only", "container")
_CONTAINER_ENV_KEYS = frozenset({
    "LANG", "LC_ALL", "LC_CTYPE", "TERM", "USER", "PYTHONUTF8",
    "PI_LEARNING_DISABLE_BACKGROUND",
})


def service_tier():
    return os.environ.get("BENCH_SERVICE_TIER", "default")


def grading_isolation(harness=None):
    """Resolve the boundary for one harness without accepting ambiguous values."""
    specific = ("BENCH_" + harness.upper() + "_GRADING_ISOLATION") if harness else None
    value = os.environ.get(specific) if specific else None
    value = value or os.environ.get("BENCH_GRADING_ISOLATION", "diagnostic-only")
    value = value.strip().lower()
    if value not in GRADING_ISOLATIONS:
        raise ValueError("grading isolation must be diagnostic-only or container")
    return value


def _container_engine():
    selected = os.environ.get("BENCH_CONTAINER_ENGINE", "docker")
    found = shutil.which(selected)
    if found is None:
        raise ValueError("missing container engine: " + selected)
    return str(Path(found).resolve())


def _container_image():
    image = os.environ.get("BENCH_CONTAINER_IMAGE", "").strip()
    if not image or any(character.isspace() for character in image):
        raise ValueError("BENCH_CONTAINER_IMAGE must name one image")
    return image


def _container_image_id(engine, image):
    result = subprocess.run([engine, "image", "inspect", "--format", "{{.Id}}", image],
                            capture_output=True, text=True, timeout=30)
    if result.returncode != 0 or not result.stdout.strip():
        raise ValueError("container image is unavailable: " + image)
    return result.stdout.strip()


def _container_binary(harness):
    variable = "BENCH_" + harness.upper() + "_CONTAINER_BINARY"
    selected = os.environ.get(variable, "").strip()
    if not selected:
        raise ValueError(variable + " is required for container grading")
    binary = Path(selected).expanduser().resolve()
    if not binary.is_file() or binary.is_symlink():
        raise ValueError(variable + " must select a regular file")
    return str(binary)


def _mount(source, target, read_only=False):
    value = "type=bind,source=" + str(Path(source).resolve()) + ",target=" + target
    if read_only:
        value += ",readonly"
    return value


def _container_version(engine, image, binary):
    command = [engine, "run", "--name", "davinci-bench-" + uuid.uuid4().hex,
               "--network", "none", "--read-only",
               "--tmpfs", "/tmp", "--cap-drop=ALL",
               "--security-opt=no-new-privileges", "--mount", _mount(binary, "/opt/harness", True),
               image, "/opt/harness", "--version"]
    result = execute(command, Path.cwd(), os.environ, 30)
    if result["exit"] != 0 or not result["cleanup_complete"]:
        raise ValueError("container executable version check failed")
    return result["stdout"].strip()


def tooling_preflight(engine=None, image=None):
    """Exercise the exact command names used by fixtures before any model call."""
    script = ("command -v python python3 pytest git bash >/dev/null; "
              "python -c 'import sys, pytest; print(sys.version); print(pytest.__version__)'; "
              "python -m pytest --version; python3 -m pytest --version; "
              "pytest --version; git --version")
    command = ["bash", "-ec", script]
    if not engine and sys.platform == "win32":
        # Native Windows runs use a Job Object. Check the names fixtures use,
        # resolved from PATH exactly as the agent will resolve them; Windows
        # Python installs usually provide `python` but not `python3`.
        missing = [name for name in ("python", "git", "bash") if shutil.which(name) is None]
        if missing:
            raise ValueError("benchmark prerequisites missing: " + ", ".join(missing))
        result = execute([shutil.which("python"), "-c",
                          "import sys, pytest; print(sys.version); print(pytest.__version__)"],
                         Path.cwd(), os.environ, 30)
        if result["exit"] != 0 or not result["cleanup_complete"]:
            raise ValueError("benchmark prerequisites missing: `python` on PATH must have pytest")
        return {"available": True, "versions": result["stdout"].strip().splitlines(),
                "lifecycle": "windows-job-object"}
    if not engine and sys.platform != "linux":
        raise ValueError("native benchmark lifecycle supervision requires Linux or Windows; use a configured container arm")
    if not engine:
        from native_supervisor import children_path_available
        if not children_path_available():
            raise ValueError("native benchmark requires /proc in its own PID namespace before any launch")
    with tempfile.TemporaryDirectory(prefix="davinci-bench-preflight-") as temporary:
        if engine:
            command = [engine, "run", "--name", "davinci-bench-" + uuid.uuid4().hex,
                       "--cidfile", str(Path(temporary) / "container.cid"),
                       "--network", "none", "--read-only", "--tmpfs", "/tmp",
                       "--cap-drop=ALL", "--security-opt=no-new-privileges", image, *command]
        result = execute(command, Path.cwd(), os.environ, 30)
    if result["exit"] == "unsupported_native_lifecycle":
        raise ValueError("native benchmark process-tree ownership is unavailable; use a configured container arm")
    if result["exit"] != 0 or not result["cleanup_complete"]:
        raise ValueError("benchmark prerequisites missing: python, python3, pytest, git and bash are required")
    return {"available": True, "versions": result["stdout"].strip().splitlines()}


def grader_preflight():
    result = subprocess.run([sys.executable, "-m", "pytest", "--version"],
                            capture_output=True, text=True, timeout=30)
    if result.returncode:
        raise ValueError("the benchmark grader's Python must have pytest installed")
    return {"python": sys.version, "pytest": result.stdout.strip()}


def stop_reason(stdout, stderr):
    errors = [stderr]
    for line in stdout.splitlines():
        try:
            event = json.loads(line)
        except ValueError:
            continue
        if not isinstance(event, dict):
            continue
        if event.get("type") in ("error", "turn.failed", "auto_retry_start", "auto_retry_end"):
            errors.append(json.dumps(event))
        elif event.get("type") == "message_end":
            message = event.get("message")
            if isinstance(message, dict) and message.get("stopReason") == "error":
                errors.append(str(message.get("errorMessage", "")))
    text = "\n".join(errors).lower()
    if any(value in text for value in ("usage limit", "rate limit", "rate_limit", "quota exceeded", "insufficient_quota")):
        return "usage_limit"
    if any(value in text for value in ("unauthorized", "invalid api key", "incorrect api key",
            "authentication failed", "credentials expired", "refresh token expired", "not logged in")):
        return "credentials"
    return None


def pin_model_store(path, model):
    """Pin one existing public Codex catalog record, without owner configuration."""
    store = json.loads(Path(path).read_text(encoding="utf-8"))
    matches = [entry for entry in store.get("providers", {}).get("openai-codex", {}).get("models", [])
               if entry.get("id") == model]
    if len(matches) != 1:
        raise ValueError("requested model must occur exactly once in the catalog")
    entry = matches[0]
    if "headers" in entry and not entry["headers"]:
        entry = {key: value for key, value in entry.items() if key != "headers"}
    allowed = {"id", "name", "api", "provider", "baseUrl", "reasoning", "input", "cost",
               "contextWindow", "maxTokens", "compat", "thinkingLevelMap"}
    if (set(entry) - allowed or entry.get("provider") != "openai-codex"
            or entry.get("api") != "openai-codex-responses"
            or entry.get("baseUrl") != "https://chatgpt.com/backend-api"):
        raise ValueError("catalog record must use the public Codex route without credentials or overrides")
    return {"providers": {"openai-codex": {"models": [entry]}}}


def model_stop_reason(stdout, expected):
    for line in stdout.splitlines():
        try:
            event = json.loads(line)
        except ValueError:
            continue
        if not isinstance(event, dict) or event.get("type") != "provider_observation":
            continue
        observation = event.get("observation", {})
        if (isinstance(observation, dict) and observation.get("kind") == "logical_start"
                and observation.get("purpose") == "coding"
                and observation.get("model") != "openai-codex/" + expected):
            return "model_mismatch"
    return None


def agent_source():
    explicit = os.environ.get("DAVINCI_CODING_AGENT_DIR", os.environ.get("PI_CODING_AGENT_DIR"))
    if explicit:
        return Path(explicit).expanduser()
    current = Path.home() / ".davinci" / "agent"
    return current if current.exists() else Path.home() / ".pi" / "agent"


def source_identity(root):
    def git_bytes(*args):
        return subprocess.run(["git", *args], cwd=root, capture_output=True, check=True).stdout
    sha = git_bytes("rev-parse", "HEAD").decode().strip()
    # Refuse dirty inputs instead of certifying a digest that cannot be rebuilt.
    if git_bytes("status", "--porcelain=v1", "--untracked-files=all"):
        raise ValueError("benchmark source must be a clean committed checkout")
    tree = git_bytes("rev-parse", "HEAD^{tree}").decode().strip()
    return {"source_sha": sha, "source_tree": tree, "source_clean": True,
            "dirty_diff_hash": CLEAN_DIFF_HASH}


def checkpoint_identity(executable, repo):
    """Read build provenance bound to copied bytes; never infer it from current HEAD."""
    executable = Path(executable)
    sidecar = executable.with_suffix(executable.suffix + ".identity.json")
    try:
        identity = json.loads(sidecar.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise ValueError("missing or invalid checkpoint identity sidecar") from error
    if not isinstance(identity, dict) or identity.get("schema_version") != 3:
        raise ValueError("checkpoint requires schema 3 clean committed green-CI build provenance")
    if identity.get("ci_verified") is not True:
        raise ValueError("checkpoint has no green CI proof")
    ci = identity.get("ci")
    release_identity.validate_ci(ci, identity.get("repository"), identity.get("source_sha", ""))
    if identity.get("ci_evidence_sha256") != hashlib.sha256(json.dumps(ci, sort_keys=True).encode()).hexdigest():
        raise ValueError("checkpoint CI proof changed after recording")
    for field, length in (("binary_sha256", 64), ("source_sha", 40), ("source_tree", 40), ("dirty_diff_hash", 64)):
        value = identity.get(field)
        if not isinstance(value, str) or re.fullmatch("[0-9a-f]{" + str(length) + "}", value) is None:
            raise ValueError("invalid checkpoint " + field)
    if identity["binary_sha256"] != file_hash(executable):
        raise ValueError("checkpoint binary changed after provenance was recorded")
    if identity.get("source_clean") is not True or identity["dirty_diff_hash"] != CLEAN_DIFF_HASH:
        raise ValueError("checkpoint was not built from clean committed source")
    result = subprocess.run(["git", "rev-parse", "--verify", identity["source_sha"] + "^{tree}"],
                            cwd=repo, capture_output=True, text=True)
    if result.returncode or result.stdout.strip() != identity["source_tree"]:
        raise ValueError("checkpoint source commit/tree is unavailable or mismatched")
    return {field: identity[field] for field in
            ("source_sha", "source_tree", "source_clean", "dirty_diff_hash")}


def parent_identity(repo, candidate_sha, base_ref, parent_sha):
    """Resolve the actual merge-base, never a supplied old ancestor as control."""
    def resolve(ref):
        result = subprocess.run(["git", "rev-parse", "--verify", "--end-of-options", ref + "^{commit}"],
                                cwd=repo, capture_output=True, text=True)
        if result.returncode:
            raise ValueError("comparison revision is unavailable")
        return result.stdout.strip()
    candidate, base, parent = map(resolve, (candidate_sha, base_ref, parent_sha))
    result = subprocess.run(["git", "merge-base", "--all", candidate, base],
                            cwd=repo, capture_output=True, text=True)
    merges = result.stdout.splitlines()
    if result.returncode or len(merges) != 1 or merges[0] != parent:
        raise ValueError("parent checkpoint must be the candidate's actual merge-base with the declared base")
    return {"candidate_source_sha": candidate, "base_source_sha": base, "merge_base_sha": parent}


def build_checkpoint(repo, destination):
    """Build and freeze bytes with before/after source identity, without model calls."""
    repo, destination = Path(repo).resolve(), Path(destination).resolve()
    if destination.is_relative_to(repo) or destination.exists():
        raise ValueError("checkpoint destination must be new and outside the repository")
    proof = release_identity.preflight(repo)
    before = proof["source"]
    with tempfile.TemporaryDirectory(prefix="davinci-bench-build-") as target:
        build = ["cargo", "build", "--locked", "--release", "-p", "davinci-coding-agent", "--bin", "davinci",
                 "--target-dir", target]
        subprocess.run(build, cwd=repo, check=True)
        if source_identity(repo) != before:
            raise ValueError("source changed during checkpoint build")
        built = Path(target) / "release" / ("davinci.exe" if os.name == "nt" else "davinci")
        destination.mkdir(parents=True, exist_ok=False)
        binary = destination / built.name
        shutil.copy2(built, binary)
        identity = release_identity.make_identity(binary, before, proof["ci"], None,
                                                 proof["version"], require_tag=False)
        identity["build_command"] = build[:-1] + ["<temporary-target>"]
        binary.with_suffix(binary.suffix + ".identity.json").write_text(
            json.dumps(identity, indent=2) + "\n", encoding="utf-8")
    return binary


def campaign_identity(root, variant, fixtures, harnesses, model, effort, settings, repo):
    """Pin executable bytes and configuration before any timed subprocess."""
    executables, identities, containers, isolations, tooling = {}, {}, {}, {}, {}
    source = source_identity(repo)
    grader = grader_preflight()
    for harness in harnesses:
        isolation = grading_isolation(harness)
        isolations[harness] = isolation
        if isolation == "container":
            if harness != "davinci":
                raise ValueError("Codex container auth and telemetry are not configured; use diagnostic-only")
            engine = _container_engine()
            image = _container_image()
            image_id = _container_image_id(engine, image)
            network = os.environ.get("BENCH_CONTAINER_NETWORK", "bridge")
            if network not in ("none", "bridge"):
                raise ValueError("unsupported container network")
            executable = _container_binary(harness)
            tooling[harness] = tooling_preflight(engine, image_id)
            version = _container_version(engine, image_id, executable)
            containers[harness] = {"image": image, "image_id": image_id,
                                   "network": network,
                                   "binary_target": "/opt/harness"}
        else:
            tooling[harness] = tooling_preflight()
            selected = os.environ.get("BENCH_" + harness.upper(), harness)
            found = shutil.which(selected)
            if found is None:
                raise ValueError("missing executable: " + harness)
            executable = str(Path(found).resolve())
            version = subprocess.run([executable, "--version"], capture_output=True,
                                     text=True, timeout=30, check=True).stdout.strip()
        if harness == "davinci" and Path(executable).is_relative_to(Path(repo).resolve()):
            raise ValueError("DaVinci executable must be an immutable copy outside the repository")
        executables[harness] = executable
        identities[harness] = {
            "binary_sha256": file_hash(executable), "version": version,
            **(checkpoint_identity(executable, repo) if harness == "davinci" else {"source_sha": None, "dirty_diff_hash": None}),
            "grading_isolation": isolation,
            "grading_assurance": "diagnostic-only",
            "credential_exposure": "readable-by-agent-process-and-tools",
            "fixture_secrecy": "public-generators-contain-graders-and-solutions",
            "effective_settings": settings if harness == "davinci" else {
                "ignore_user_config": True, "effort": effort, "service_tier": service_tier(),
                "telemetry": "local-sanitized-otlp-logs-and-traces"}}
    return {"schema_version": 2, "campaign": Path(root).name, "variant": variant,
            "fixture_hash": fixtures["fixture_hash"], "model": model,
            "effort_policy": effort, "service_tier": service_tier(),
            "executables": executables, "identities": identities,
            "grading_isolation": isolations, "containers": containers,
            "agent_dir": str((Path(root) / "_agent").resolve()),
            "os": platform.platform(), "cpu": platform.processor(), "python": sys.version,
            "tooling_preflight": tooling, "grader_preflight": grader,
            "host": platform.node(), "runner_source": source, "resolved_model_identity": None,
            "promotion_eligible": False}


def container_command(command, workdir, env, campaign, harness):
    """Limit host mounts. This is NOT a secret boundary from the agent's tools."""
    if campaign.get("grading_isolation", {}).get(harness) != "container":
        return command
    config = campaign.get("containers", {}).get(harness)
    if not isinstance(config, dict):
        raise ValueError("container campaign is missing harness configuration")
    engine = _container_engine()
    image = config.get("image")
    image_id = config.get("image_id")
    if not isinstance(image, str) or not image or not isinstance(image_id, str) or not image_id:
        raise ValueError("container campaign has invalid image identity")
    if _container_image_id(engine, image) != image_id:
        raise ValueError("container image changed during campaign")
    binary = campaign["executables"].get(harness)
    agent_dir = campaign.get("agent_dir")
    if not isinstance(binary, str) or not Path(binary).is_file():
        raise ValueError("container executable is unavailable")
    if not isinstance(agent_dir, str) or not Path(agent_dir).is_dir():
        raise ValueError("container agent directory is unavailable")
    workdir = Path(workdir).resolve()
    if not workdir.is_dir():
        raise ValueError("container worktree is unavailable")
    child_env = ["-e", "HOME=/root", "-e", "PWD=/workspace", "-e", "TMP=/tmp",
                 "-e", "TEMP=/tmp", "-e", "TMPDIR=/tmp",
                 "-e", "DAVINCI_CODING_AGENT_DIR=/agent",
                 "-e", "PI_CODING_AGENT_DIR=/agent"]
    for key in sorted(_CONTAINER_ENV_KEYS):
        value = env.get(key)
        if value is not None:
            child_env.extend(["-e", key + "=" + str(value)])
    network = config.get("network", "bridge")
    if network not in ("none", "bridge"):
        raise ValueError("unsupported container network")
    executable_args = list(command[1:])
    return [engine, "run", "--name", "davinci-bench-" + uuid.uuid4().hex,
            "--cidfile", str(workdir) + ".cid", "--workdir", "/workspace", "--network", network,
            "--read-only", "--tmpfs", "/tmp", "--tmpfs", "/root",
            "--cap-drop=ALL", "--security-opt=no-new-privileges",
            "--mount", _mount(workdir, "/workspace"),
            "--mount", _mount(agent_dir, "/agent", True),
            "--mount", _mount(binary, "/opt/harness", True),
            *child_env, image_id, "/opt/harness", *executable_args]


def _container_target(command):
    if len(command) > 3 and command[1] == "run" and "--name" in command:
        name = command[command.index("--name") + 1]
        if name.startswith("davinci-bench-"):
            return command[0], name
    return None


def _remove_container(target):
    """Kill the actual container and confirm absence, including detached children."""
    if target is None:
        return True
    engine, name = target
    try:
        subprocess.run([engine, "rm", "--force", name], capture_output=True, timeout=30)
        inspected = subprocess.run([engine, "container", "inspect", name],
                                   capture_output=True, text=True, timeout=10)
        # An unreachable daemon is not evidence that the container is gone.
        return (inspected.returncode != 0 and
                "no such container" in inspected.stderr.lower())
    except (OSError, subprocess.SubprocessError):
        return False


def _kill_process(process):
    if os.name == "nt":
        subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=10)
    else:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
    if process.poll() is None:
        process.kill()


def _container_creation_acknowledged(command, code):
    if "--cidfile" not in command:
        # A successful synchronous client exit also acknowledges completion.
        # Abrupt/failed exits and timeouts need the daemon's explicit create ID.
        return type(code) is int and code == 0
    try:
        cid = Path(command[command.index("--cidfile") + 1]).read_text(encoding="utf-8").strip()
        return re.fullmatch(r"[0-9a-f]{64}", cid) is not None
    except (OSError, ValueError, IndexError):
        return False


def _execute_container(command, workdir, env, timeout, target):
    try:
        process = subprocess.Popen(command, cwd=workdir, env=env,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            text=True, encoding="utf-8", errors="replace",
            start_new_session=os.name != "nt")
    except OSError as error:
        return {"exit": "launch_error", "stdout": "", "stderr": type(error).__name__,
                "cleanup_complete": _remove_container(target)}
    try:
        try:
            stdout, stderr = process.communicate(timeout=timeout)
            code = process.returncode
        except subprocess.TimeoutExpired:
            # Removing docker's client process alone leaves the container alive.
            _remove_container(target)
            _kill_process(process)
            stdout, stderr = process.communicate(timeout=10)
            # The client can still be submitting a create request during the
            # first removal. Check again after reaping it, before any grading.
            code = "timeout"
        removed = _remove_container(target)
        # This applies to abrupt client death as well as our own timeout. An
        # absent name cannot rule out a create still pending in the daemon.
        cleanup_complete = removed and _container_creation_acknowledged(command, code)
    except BaseException:
        _remove_container(target)
        _kill_process(process)
        process.communicate(timeout=10)
        raise
    return {"exit": code, "stdout": stdout, "stderr": stderr,
            "cleanup_complete": cleanup_complete}


def _execute_native(command, workdir, env, timeout):
    if sys.platform == "win32":
        import windows_job
        try:
            return windows_job.run(command, workdir, env, timeout)
        except OSError as error:
            return {"exit": "unsupported_native_lifecycle", "stdout": "",
                    "stderr": f"Windows job object unavailable: {error}",
                    "cleanup_complete": False}
    if sys.platform != "linux":
        return {"exit": "unsupported_native_lifecycle", "stdout": "", "stderr":
                "native benchmark lifecycle supervision requires Linux", "cleanup_complete": False}
    with tempfile.TemporaryDirectory(prefix="davinci-bench-native-") as temporary:
        report = Path(temporary) / "result.json"
        supervised = [sys.executable, str(Path(__file__).with_name("native_supervisor.py")),
                      "--report", str(report), "--timeout", str(timeout), "--", *command]
        try:
            process = subprocess.Popen(supervised, cwd=workdir, env=env,
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                encoding="utf-8", errors="replace", start_new_session=True)
        except OSError as error:
            return {"exit": "launch_error", "stdout": "", "stderr": type(error).__name__,
                    "cleanup_complete": True}
        try:
            stdout, stderr = process.communicate(timeout=timeout + 15)
        except subprocess.TimeoutExpired:
            process.terminate()
            try:
                stdout, stderr = process.communicate(timeout=15)
            except subprocess.TimeoutExpired:
                _kill_process(process)
                try:
                    stdout, stderr = process.communicate(timeout=5)
                except subprocess.TimeoutExpired:
                    stdout, stderr = "", "native supervisor could not be reaped"
            return {"exit": "supervisor_timeout", "stdout": stdout, "stderr": stderr,
                    "cleanup_complete": False}
        except BaseException:
            # Give the owner a chance to reap its tree. Without its completed
            # report, the persistent campaign record deliberately stays dirty.
            process.terminate()
            try:
                process.communicate(timeout=15)
            except subprocess.TimeoutExpired:
                _kill_process(process)
                process.communicate(timeout=5)
            raise
        try:
            result = json.loads(report.read_text(encoding="utf-8"))
            valid = (process.returncode == 0 and isinstance(result, dict)
                     and result.get("schema_version") == 1
                     and (type(result.get("exit")) is int or result.get("exit") in
                          ("timeout", "interrupted", "launch_error", "unsupported_native_lifecycle")))
        except (OSError, ValueError, TypeError):
            result, valid = {}, False
        return {"exit": result.get("exit", "supervisor_failed") if valid else "supervisor_failed",
                "stdout": stdout, "stderr": stderr,
                "cleanup_complete": valid and result.get("cleanup_complete") is True}


def execute(command, workdir, env, timeout):
    """Measure through proven process-tree/container cleanup before any grading."""
    started_at = datetime.now(timezone.utc).isoformat()
    started = time.perf_counter()
    target = _container_target(command)
    owner = _campaign_owner
    token = owner.begin(target, workdir) if owner is not None else None
    measured = (_execute_container(command, workdir, env, timeout, target) if target
                else _execute_native(command, workdir, env, timeout))
    if owner is not None and measured["cleanup_complete"] is True:
        owner.complete(token)
    return {**measured, "wall_s": time.perf_counter() - started,
            "started_at": started_at, "finished_at": datetime.now(timezone.utc).isoformat()}


def create_campaign(root, manifest):
    """Exclusive creation prevents appending new variants to historical rows."""
    root = Path(root)
    root.mkdir(parents=True, exist_ok=False, mode=0o700)
    (root / "campaign.json").write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


def isolate_settings(source, target, settings):
    """Restrict host permissions; tools in the same agent remain able to read auth."""
    source, target = Path(source), Path(target)
    auth = source / "auth.json"
    if not auth.is_file():
        raise ValueError("credential file is unavailable in the selected agent directory")
    if auth.is_symlink():
        raise ValueError("linked credential files require an explicit resolved source")
    target.mkdir(parents=True, exist_ok=False, mode=0o700)
    shutil.copyfile(auth, target / "auth.json")
    (target / "auth.json").chmod(0o600)
    (target / "settings.json").write_text(
        json.dumps(settings, indent=2) + "\n", encoding="utf-8")
    return {"directory": str(target.resolve()), "settings": settings}


def _campaign_lock_path():
    system_temp = (Path(os.environ.get("SystemRoot", r"C:\Windows")) / "Temp"
                   if os.name == "nt" else Path("/tmp"))
    return system_temp / "davinci-benchmark-campaign.lock"


class _CampaignOwner:
    def __init__(self, lock, campaign):
        self.lock = lock
        self.record = {"schema_version": 1, "pid": os.getpid(),
                       "campaign": str(Path(campaign).resolve()), "status": "active",
                       "started_at": datetime.now(timezone.utc).isoformat(), "in_flight": []}
        self.persist()

    def persist(self):
        # Overwrite in place and only then shorten the file. Truncating first
        # would leave an empty file after a crash mid-write, which the next
        # campaign would read as a clean record. A torn overwrite is invalid
        # JSON instead, which is treated as unresolved ownership.
        data = json.dumps(self.record)
        self.lock.seek(0, os.SEEK_END)
        previous = self.lock.tell()
        self.lock.seek(0)
        self.lock.write(data.ljust(previous))
        self.lock.flush()
        os.fsync(self.lock.fileno())
        self.lock.truncate(len(data))
        self.lock.flush()
        os.fsync(self.lock.fileno())

    def begin(self, target, workdir):
        if self.record["in_flight"]:
            raise RuntimeError("benchmark campaign has unresolved process cleanup")
        token = uuid.uuid4().hex
        self.record["in_flight"] = [{"id": token, "workspace": str(Path(workdir).resolve()),
                                     "container": target,
                                     "kind": ("container" if target else
                                              "windows-job" if os.name == "nt" else "native"),
                                     "started_at": datetime.now(timezone.utc).isoformat()}]
        # Persist intent before Popen, closing the crash window before recording
        # a PID/CID. An unresolved intent is sufficient to block the next campaign.
        self.persist()
        return token

    def complete(self, token):
        if self.record["in_flight"][0]["id"] != token:
            raise RuntimeError("benchmark lifecycle ownership changed")
        self.record["in_flight"] = []
        self.persist()

    def finish(self):
        if not self.record["in_flight"]:
            self.record["status"] = "clean"
            self.persist()


_WINDOWS_LOCK_OFFSET = 1 << 20


def _pid_alive(pid):
    if type(pid) is not int or pid <= 0:
        return True  # Unknown owner: never assume it is gone.
    if os.name == "nt":
        import ctypes
        from ctypes import wintypes
        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel32.OpenProcess.restype = wintypes.HANDLE
        handle = kernel32.OpenProcess(0x1000, False, pid)  # QUERY_LIMITED_INFORMATION
        if not handle:
            # Access denied means some process has this PID; only a missing
            # process (ERROR_INVALID_PARAMETER) proves the owner is gone.
            return ctypes.get_last_error() != 87
        try:
            code = wintypes.DWORD()
            if not kernel32.GetExitCodeProcess(handle, ctypes.byref(code)):
                return True
            return code.value == 259  # STILL_ACTIVE
        finally:
            kernel32.CloseHandle(handle)
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except OSError:
        return True
    return True


def _reconcile_abandoned(record):
    """Clear a dead orchestrator's record only when its work is proven gone.

    A container is proven gone by force removal plus a "no such container"
    inspection. A Windows native run is proven gone by its kill-on-close Job
    Object: the kernel killed the tree when the dead owner's handle closed.
    Linux native runs and unreadable records still need an operator.
    """
    if not isinstance(record, dict) or record.get("schema_version") != 1:
        return False
    in_flight = record.get("in_flight")
    if not isinstance(in_flight, list) or _pid_alive(record.get("pid")):
        return False
    for entry in in_flight:
        if not isinstance(entry, dict):
            return False
        kind = entry.get("kind")
        if kind == "container":
            target = entry.get("container")
            if not (isinstance(target, list) and len(target) == 2
                    and all(isinstance(part, str) and part for part in target)
                    and target[1].startswith("davinci-bench-")):
                return False
            if not _remove_container(tuple(target)):
                return False
        elif kind != "windows-job":
            return False
    return True


@contextmanager
def campaign_lock(campaign):
    """Machine lock plus durable launch intent, which survives orchestrator death.

    The kernel lock alone releases on a crash while workers/containers may live.
    Never admit a new campaign over unresolved or corrupt ownership records.
    Keep the inode: unlinking it could split concurrent kernel ownership.
    """
    global _campaign_owner
    path = _campaign_lock_path()
    flags = os.O_RDWR | os.O_CREAT | getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open(path, flags, 0o600)
    except OSError as error:
        raise RuntimeError("cannot acquire the machine benchmark campaign lock") from error
    with os.fdopen(descriptor, "r+") as lock:
        try:
            if os.name == "nt":
                import msvcrt
                # Lock one byte far past the record, so the record itself stays
                # readable by operators and tests while the lock is held.
                lock.seek(_WINDOWS_LOCK_OFFSET)
                msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError as error:
            raise RuntimeError("another benchmark campaign is active on this machine") from error
        owner = None
        try:
            lock.seek(0)
            previous = lock.read().strip()
            if previous:
                try:
                    record = json.loads(previous)
                    safe = (isinstance(record, dict) and record.get("schema_version") == 1
                            and record.get("status") == "clean"
                            and record.get("in_flight") == [])
                except ValueError:
                    record, safe = None, False
                if not safe and not _reconcile_abandoned(record):
                    raise RuntimeError("unresolved benchmark campaign ownership: " + str(path) +
                                       "; reconcile prior workers and containers before a new campaign")
            owner = _CampaignOwner(lock, campaign)
            _campaign_owner = owner
            yield {"scope": "machine", "mechanism": "kernel-lock-and-durable-launch-intent-v1"}
        finally:
            if owner is not None:
                try:
                    owner.finish()
                finally:
                    _campaign_owner = None
            if os.name == "nt":
                lock.seek(_WINDOWS_LOCK_OFFSET)
                msvcrt.locking(lock.fileno(), msvcrt.LK_UNLCK, 1)
            else:
                fcntl.flock(lock, fcntl.LOCK_UN)


def controlled_environment(inherited, agent_dir):
    """Use copied credentials and declared settings, not inherited experiment knobs."""
    excluded = {"OPENAI_API_KEY", "OPENAI_BASE_URL", "OPENAI_ORG_ID",
                "OPENAI_PROJECT_ID", "ANTHROPIC_API_KEY", "PYTHONPATH",
                "PYTHONSTARTUP", "PYTHONOPTIMIZE"}
    result = {key: value for key, value in inherited.items()
              if not key.upper().startswith(("BENCH_", "DAVINCI_", "PI_", "OTEL_"))
              and key.upper() not in excluded}
    result.update({"DAVINCI_CODING_AGENT_DIR": str(Path(agent_dir).resolve()),
                   "PI_CODING_AGENT_DIR": str(Path(agent_dir).resolve()),
                   "PI_LEARNING_DISABLE_BACKGROUND": "1", "PYTHONUTF8": "1"})
    return result


def select_tasks(task_set, requested, large_manifest=None):
    members = list(LEGACY_TASKS)
    if task_set not in ("legacy", "large", "all"):
        raise ValueError("unknown task set")
    if task_set != "legacy":
        if large_manifest is None:
            raise ValueError("large tasks require a frozen manifest")
        validate_large_manifest(large_manifest)
        large = large_manifest.get("tasks")
        members = large if task_set == "large" else members + large
    tasks = members if not requested or requested == ["all"] else requested
    if not tasks or any(t not in members for t in tasks) or len(set(tasks)) != len(tasks):
        raise ValueError("requested tasks must be unique members of the frozen task set")
    return list(tasks)


def validate_large_manifest(manifest):
    """Validate the frozen large-stratum contract before selecting tasks."""
    if not isinstance(manifest, dict) or manifest.get("schema_version") != 1:
        raise ValueError("invalid large fixture manifest")
    if manifest.get("task_set") != "large":
        raise ValueError("large fixture manifest has the wrong task set")
    if manifest.get("hash_order") != "relative-posix-codepoint-v1":
        raise ValueError("large manifest needs the portable relative-posix-codepoint-v1 hash order; regenerate before a new campaign")
    tasks = manifest.get("tasks")
    if (not isinstance(tasks, list) or not tasks
            or any(not isinstance(task, str) or Path(task).name != task
                   or task in (".", "..") or task in LEGACY_TASKS for task in tasks)
            or len(set(tasks)) != len(tasks)):
        raise ValueError("invalid large task membership")
    fixtures = manifest.get("fixtures")
    if not isinstance(fixtures, dict) or set(fixtures) != set(tasks):
        raise ValueError("large manifest fixtures do not match membership")
    for task in tasks:
        entry = fixtures[task]
        if not isinstance(entry, dict) or entry.get("task_set") != "large":
            raise ValueError("missing large fixture metadata: " + task)
        allowed = entry.get("allowed")
        if (not isinstance(allowed, list) or not allowed
                or any(not isinstance(path, str) or Path(path).name in (".", "..")
                       or Path(path).is_absolute() or ".." in Path(path).parts
                       for path in allowed)):
            raise ValueError("invalid large allowlist: " + task)
        for field in ("public_hash", "reference_solution_hash", "hidden_grader_hash"):
            value = entry.get(field)
            if (not isinstance(value, str) or len(value) != 64
                    or re.fullmatch(r"[0-9a-f]{64}", value) is None):
                raise ValueError("invalid large fixture hash: " + task)
        if not isinstance(entry.get("public_verification"), str) or not entry["public_verification"].strip():
            raise ValueError("missing public verification: " + task)
    return True


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser(description="Build an immutable clean-source DaVinci checkpoint (no model calls)")
    parser.add_argument("--repo", required=True)
    parser.add_argument("--output", required=True)
    arguments = parser.parse_args()
    print(build_checkpoint(arguments.repo, arguments.output))
