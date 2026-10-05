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


def write_private(path: Path, text: str) -> None:
    """Credential files are owner-only (0600); write_text would use the umask."""
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
        handle.write(text)


def user_auth_path() -> Path:
    return Path.home() / ".pi/agent/auth.json"


def lend(auth: Path) -> dict:
    lent = json.loads(user_auth_path().read_text(encoding="utf-8"))[PROVIDER]
    write_private(auth, json.dumps({PROVIDER: lent}))
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
    temporary.unlink(missing_ok=True)  # O_CREAT keeps an old file's mode.
    write_private(temporary, json.dumps(store, indent=2))
    os.replace(temporary, path)
    return True
