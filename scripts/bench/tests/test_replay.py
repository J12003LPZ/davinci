import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from replay import recording


class ReplayTests(unittest.TestCase):
    def test_tool_results_are_preserved_before_the_next_response(self):
        message = {"id": "next", "role": "assistant", "content": []}
        result = {"type": "tool_execution_end", "toolCallId": "call1",
                  "isError": True, "result": "exit 1"}
        done = {"type": "message_update", "assistantMessageEvent": {"type": "done", "message": message}}
        frames = recording("\n".join(map(json.dumps, [result, done])), "C:/old", "C:/new")
        self.assertEqual(frames[0]["results_before"], [{"id": "call1", "is_error": True, "result": "exit 1"}])
        with self.assertRaises(ValueError):
            recording("\n".join(map(json.dumps, [dict(result, isError=None), done])), "C:/old", "C:/new")

    def test_only_model_responses_are_replayed_with_prior_reminders(self):
        assistant = {"id": "a1", "role": "assistant", "model": "fixture",
                     "content": [{"type": "text", "text": "done"}], "stopReason": "stop"}
        events = [{"type": "message_end", "message": {"role": "assistant",
                   "davinciHarnessVerification": True}},
                  {"type": "message_update", "assistantMessageEvent": {
                      "type": "done", "message": assistant}},
                  {"type": "message_end", "message": {
                      "davinciCapabilityReminder": "verification_required"}},
                  {"type": "message_update", "assistantMessageEvent": {
                      "type": "done", "message": dict(assistant, id="a2")}}]
        frames = recording("\n".join(map(json.dumps, events)), "C:/old", "C:/new")
        self.assertEqual(len(frames), 2)
        self.assertEqual(frames[0]["reminders_before"], [])
        self.assertEqual(frames[1]["reminders_before"], ["verification_required"])

    def test_remap_only_tool_arguments_and_reject_duplicate_model_ids(self):
        message = {"id": "a1", "role": "assistant", "model": "fixture", "content": [
            {"type": "text", "text": "C:/old stays in prose"},
            {"type": "toolCall", "name": "read", "id": "call1",
             "arguments": {"path": "C:/old/file.py"}}]}
        event = {"type": "message_update", "assistantMessageEvent": {"type": "done", "message": message}}
        line = json.dumps(event)
        frames = recording(line, "C:/old", "C:/new")
        self.assertEqual(frames[0]["assistant"]["content"][1]["arguments"]["path"], "C:/new/file.py")
        self.assertEqual(frames[0]["assistant"]["content"][0]["text"], "C:/old stays in prose")
        with self.assertRaises(ValueError):
            recording(line + "\n" + line, "C:/old", "C:/new")
