import importlib.util
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

SPEC = importlib.util.spec_from_file_location("live_auth", Path(__file__).parents[2] / "live_auth.py")
live_auth = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(live_auth)

LOGIN = {"type": "oauth", "access": "a1", "refresh": "r1", "expires": 1}
ROTATED = {"type": "oauth", "access": "a2", "refresh": "r2", "expires": 2}


class LiveAuthTests(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        self.store = self.root / "user-auth.json"
        self.store.write_text(json.dumps({"openai-codex": LOGIN, "other": {"key": "k"}}))
        self.auth = self.root / "eval-auth.json"
        patcher = mock.patch.object(live_auth, "user_auth_path", return_value=self.store)
        patcher.start()
        self.addCleanup(patcher.stop)

    def stored(self):
        return json.loads(self.store.read_text())

    def test_lends_only_the_codex_login(self):
        lent = live_auth.lend(self.auth)
        self.assertEqual(lent, LOGIN)
        self.assertEqual(json.loads(self.auth.read_text()), {"openai-codex": LOGIN})

    def test_a_rotated_login_is_given_back_and_other_providers_are_kept(self):
        lent = live_auth.lend(self.auth)
        self.auth.write_text(json.dumps({"openai-codex": ROTATED}))
        self.assertTrue(live_auth.give_back(self.auth, lent))
        self.assertEqual(self.stored(), {"openai-codex": ROTATED, "other": {"key": "k"}})

    def test_an_unchanged_login_writes_nothing(self):
        lent = live_auth.lend(self.auth)
        before = self.store.read_text()
        self.assertFalse(live_auth.give_back(self.auth, lent))
        self.assertEqual(self.store.read_text(), before)

    def test_a_newer_user_login_is_not_overwritten(self):
        lent = live_auth.lend(self.auth)
        newer = {"type": "oauth", "access": "a3", "refresh": "r3", "expires": 3}
        self.store.write_text(json.dumps({"openai-codex": newer}))
        self.auth.write_text(json.dumps({"openai-codex": ROTATED}))
        self.assertFalse(live_auth.give_back(self.auth, lent))
        self.assertEqual(self.stored()["openai-codex"], newer)

    @unittest.skipIf(os.name == "nt", "POSIX permission bits")
    def test_credential_files_are_owner_only(self):
        self.store.chmod(0o600)
        lent = live_auth.lend(self.auth)
        self.assertEqual(self.auth.stat().st_mode & 0o777, 0o600)
        self.auth.write_text(json.dumps({"openai-codex": ROTATED}))
        live_auth.give_back(self.auth, lent)
        self.assertEqual(self.store.stat().st_mode & 0o777, 0o600)

    def test_a_missing_eval_store_is_ignored(self):
        lent = live_auth.lend(self.auth)
        self.auth.unlink()
        self.assertFalse(live_auth.give_back(self.auth, lent))


if __name__ == "__main__":
    unittest.main()
