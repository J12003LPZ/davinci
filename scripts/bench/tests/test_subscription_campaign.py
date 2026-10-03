import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import subscription_campaign as subscription
import runner
import bench
from unittest.mock import patch


class SubscriptionCampaignTests(unittest.TestCase):
    def policy(self):
        return {"schema_version": 1, "billing": "subscription-only",
                "model": "gpt-6-luna", "effort": "high", "max_requests": 24,
                "max_tasks": 3, "max_wall_seconds": 1200}

    def settings(self):
        return {"transport": "sse", "effortPolicy": "fixed",
                "retry": {"enabled": False, "provider": {"maxRetries": 0}}}

    def test_freezes_request_and_wall_caps_without_usd_or_output_contract(self):
        with tempfile.TemporaryDirectory() as temporary:
            config = subscription.prepare(self.policy(), temporary, 3, ["davinci"],
                                          "gpt-6-luna", "high", self.settings(), now=10)
            self.assertEqual(config["root_budget"]["limits"]["max_requests"], 24)
            self.assertEqual(config["root_budget"]["limits"]["deadline_unix_ms"], 1210000)
            self.assertIsNone(config["root_budget"]["limits"]["max_cost_microusd"])
            self.assertIsNone(config["root_budget"]["limits"]["max_output_tokens"])
            self.assertEqual(subscription.remaining_seconds(config, now=1200), 10)
            with self.assertRaises(ValueError):
                subscription.remaining_seconds(config, now=1210)

    def test_refuses_billing_model_effort_task_and_settings_changes(self):
        for field, value in (("billing", "api"), ("model", "other"), ("effort", "low"),
                             ("max_requests", 0), ("max_tasks", True), ("max_wall_seconds", -1),
                             ("max_usd", 1)):
            policy = dict(self.policy(), **{field: value})
            with self.subTest(field=field), self.assertRaises(ValueError):
                subscription.prepare(policy, ".", 3, ["davinci"], "gpt-6-luna", "high", self.settings())
        for count, harness, model, effort, settings in (
            (4, ["davinci"], "gpt-6-luna", "high", self.settings()),
            (3, ["codex"], "gpt-6-luna", "high", self.settings()),
            (3, ["davinci"], "other", "high", self.settings()),
            (3, ["davinci"], "gpt-6-luna", "low", self.settings()),
            (3, ["davinci"], "gpt-6-luna", "high", {}),
        ):
            with self.assertRaises(ValueError):
                subscription.prepare(self.policy(), ".", count, harness, model, effort, settings)

    def test_incomplete_failed_or_halted_run_stops_campaign(self):
        with tempfile.TemporaryDirectory() as temporary:
            config = subscription.prepare(self.policy(), temporary, 3, ["davinci"],
                                          "gpt-6-luna", "high", self.settings())
            ledger = Path(config["root_budget"]["ledger"])
            self.assertEqual(subscription.stop_reason(config, 0), "subscription_missing_ledger")
            identity = {"schema_version": 1, "root": config["root_budget"]["root_id"],
                        "limits": config["root_budget"]["limits"]}
            ledger.write_text(json.dumps(dict(identity, halted=False, reservations={
                "a": {"disposition": "committed"}})), encoding="utf-8")
            self.assertIsNone(subscription.stop_reason(config, 0))
            self.assertEqual(subscription.stop_reason(config, 1), "subscription_process_failed")
            for state in ("pending", "unknown"):
                ledger.write_text(json.dumps(dict(identity, halted=False, reservations={
                    "a": {"disposition": state}})), encoding="utf-8")
                self.assertEqual(subscription.stop_reason(config, 0), "subscription_unresolved_request")

    def test_each_launch_needs_fresh_receipts_and_preserves_prior_ledger(self):
        with tempfile.TemporaryDirectory() as temporary:
            config = subscription.prepare(self.policy(), temporary, 3, ["davinci"],
                                          "gpt-6-luna", "high", self.settings())
            ledger = Path(config["root_budget"]["ledger"])
            before = subscription.snapshot(config)
            value = {"schema_version": 1, "root": config["root_budget"]["root_id"],
                     "limits": config["root_budget"]["limits"], "halted": False,
                     "reservations": {"a": {"disposition": "committed"}}}
            ledger.write_text(json.dumps(value), encoding="utf-8")
            self.assertIsNone(subscription.stop_reason(config, 0, before))
            before = subscription.snapshot(config)
            self.assertEqual(subscription.stop_reason(config, 0, before), "subscription_no_new_request")
            value["reservations"] = {"b": {"disposition": "committed"}}
            ledger.write_text(json.dumps(value), encoding="utf-8")
            self.assertEqual(subscription.stop_reason(config, 0, before), "subscription_ledger_replaced")
            ledger.unlink()
            self.assertEqual(subscription.stop_reason(config, 0, before), "subscription_missing_ledger")
            with self.assertRaises(ValueError):
                subscription.snapshot(config)

    def test_subscription_credentials_exclude_other_accounts_and_keys(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source, target = root / "source", root / "target"
            source.mkdir()
            oauth = {"type": "oauth", "access": "synthetic-access", "refresh": "synthetic-refresh", "expires": 1}
            auth = {"openai-codex": oauth, "openai": {"type": "api_key", "key": "synthetic-key"}}
            (source / "auth.json").write_text(json.dumps(auth), encoding="utf-8")
            runner.isolate_settings(source, target, self.settings(), subscription_only=True)
            self.assertEqual(json.loads((target / "auth.json").read_text()), {"openai-codex": oauth})
            auth["openai-codex"]["type"] = "api_key"
            (source / "auth.json").write_text(json.dumps(auth), encoding="utf-8")
            with self.assertRaises(ValueError):
                runner.isolate_settings(source, root / "invalid", self.settings(), subscription_only=True)

    def test_subscription_environment_drops_api_credentials(self):
        env = runner.controlled_environment({"PATH": "system", "GEMINI_API_KEY": "synthetic",
            "OPENROUTER_API_KEY": "synthetic", "AZURE_OPENAI_API_KEY": "synthetic"}, ".", subscription_only=True)
        self.assertEqual(env["PATH"], "system")
        self.assertFalse(any(key.endswith("API_KEY") for key in env))

    def test_grading_timeout_is_explicit_and_bounded_by_campaign_deadline(self):
        self.assertEqual(bench.grading_timeout(None), 120)
        self.assertEqual(bench.grading_timeout({"grading_timeout_seconds": 900}), 900)
        campaign = {"grading_timeout_seconds": 900, "subscription": {"fixture": True}}
        with patch.object(subscription, "remaining_seconds", return_value=2):
            self.assertEqual(bench.grading_timeout(campaign), 2)
        with patch.object(subscription, "remaining_seconds", side_effect=ValueError("exhausted")):
            with self.assertRaises(ValueError):
                bench.grading_timeout(campaign)


if __name__ == "__main__":
    unittest.main()
