"""Lend the user's openai-codex login to a disposable eval config, then return it.

Sign in with ChatGPT rotates the refresh token on every refresh. A temporary
copy that refreshes leaves the user's own auth.json holding a dead token, and
the next real run reports that the sign-in expired. `lend` copies only the
openai-codex entry; `give_back` writes a rotated entry back, unless the user's
store changed meanwhile.
"""
import json
import os
from pathlib import Path

PROVIDER = "openai-codex"


def user_auth_path() -> Path:
    return Path.home() / ".pi/agent/auth.json"


def lend(auth: Path) -> dict:
    lent = json.loads(user_auth_path().read_text(encoding="utf-8"))[PROVIDER]
    auth.write_text(json.dumps({PROVIDER: lent}), encoding="utf-8")
    return lent


def give_back(auth: Path, lent: dict) -> bool:
    try:
        current = json.loads(auth.read_text(encoding="utf-8")).get(PROVIDER)
    except (OSError, ValueError):
        return False
    if not current or current == lent:
        return False
    path = user_auth_path()
    store = json.loads(path.read_text(encoding="utf-8"))
    if store.get(PROVIDER) != lent:
        return False  # A newer login replaced it while the eval ran; keep that.
    store[PROVIDER] = current
    temporary = path.with_name(path.name + ".eval-tmp")
    temporary.write_text(json.dumps(store, indent=2), encoding="utf-8")
    os.replace(temporary, path)
    return True
