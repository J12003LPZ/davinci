"""A diagnostic cache_drop event must not invalidate request metrics."""
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from observations import activity


def observation(kind, request="req-1", **fields):
    body = {"schema_version": 1, "kind": kind, "logical_request_id": request, "root_id": None,
            "actor_id": None, "parent_actor_id": None, "attempt_id": None, "purpose": "coding",
            "model": "openai-codex/fixture", "status": "completed"}
    body.update(fields)
    return {"type": "provider_observation", "observation": body}


def cache_drop(request):
    # Shaped like ProviderAttemptObservation::for_cache_drop: a fresh request id
    # that never has a start or end, and the drop record in camelCase.
    return observation("cache_drop", request=request, cache_drop={
        "requestIndex": 2, "prevCacheRead": 9000, "nextCacheRead": 0,
        "wirePrefixDigest": "abc123", "route": "responses", "reason": "providerOrUnknown"})


class CacheDropTests(unittest.TestCase):
    def run_events(self, *extra):
        return activity([
            observation("logical_start"), observation("attempt_start", attempt_id=1),
            observation("attempt_end", attempt_id=1), observation("logical_end"), *extra])

    def test_cache_drop_is_neither_a_request_nor_telemetry_loss(self):
        plain, dropped = self.run_events(), self.run_events(cache_drop("drop-1"))
        self.assertTrue(dropped["request_metrics_available"])
        self.assertTrue(dropped["request_metrics_complete"])
        self.assertEqual(dropped["requests"], 1)
        self.assertEqual(dropped["provider_attempts"], 1)
        self.assertEqual(dropped, plain)

    def test_unknown_kind_still_invalidates_metrics(self):
        events = self.run_events(observation("something_new", request="x"))
        self.assertFalse(events["request_metrics_available"])


if __name__ == "__main__":
    unittest.main()
