"""Droid-family harness names and their fixed configuration (pure, no I/O).

``droid`` keeps the process-wide ``BENCH_DROID_TOOL_EXECUTION_MODE`` behaviour.
``droid-direct`` and ``droid-script`` pin the hidden ``--tool-execution-mode``
flag so a row says which Droid arm it is and a three-arm campaign can be
scheduled (preregistration G4). All three run the binary named by ``BENCH_DROID``.
"""
import os

DROID_HARNESSES = ("droid", "droid-direct", "droid-script")
FIXED_MODES = {"droid-direct": "direct-only", "droid-script": "script-only"}
AUTONOMY_LEVELS = ("low", "medium", "high")
# Droid 1.1.0 documents no reasoning-effort flag for `droid exec`; effort comes
# from the proxy model configuration, which the bench must not write.
REASONING_EFFORT = "not forced"


def is_droid(harness):
    return harness in DROID_HARNESSES


def fixed_mode(harness):
    """The tool-execution mode a harness name pins, or None for plain ``droid``."""
    return FIXED_MODES.get(harness)


def droid_model(default_model):
    """``BENCH_DROID_MODEL`` (the proxy alias), falling back to ``BENCH_MODEL``."""
    return os.environ.get("BENCH_DROID_MODEL") or default_model


def droid_auto():
    """Droid autonomy level from ``BENCH_DROID_AUTO`` (default ``high``).

    ``--auto`` is autonomy, not reasoning effort, so ``BENCH_EFFORT`` is never used.
    """
    value = (os.environ.get("BENCH_DROID_AUTO") or "high").strip().lower()
    if value not in AUTONOMY_LEVELS:
        raise ValueError("BENCH_DROID_AUTO must be low, medium or high")
    return value


def env_name(harness):
    """Environment-variable stem for a harness (hyphens are not portable)."""
    return harness.upper().replace("-", "_")


def binary_env(harness):
    """Executable override variable: every Droid arm uses ``BENCH_DROID``."""
    return "BENCH_DROID" if is_droid(harness) else "BENCH_" + env_name(harness)
