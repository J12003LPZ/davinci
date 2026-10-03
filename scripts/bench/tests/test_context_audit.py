import json
import sys
import tempfile
import unittest
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from context_audit import calls, leaves, listing_only

class ContextAuditTests(unittest.TestCase):
    def test_nested_batch_aliases(self):
        self.assertEqual(list(leaves("functions.batch", {"operations": [
            {"tool": "functions.bash", "args": {"command": "pwd"}}
        ]})), [("bash", {"command": "pwd"})])

    def test_listing_rejects_mutation_and_mixed_reads(self):
        for command in ["pwd && ls -la && find .. -name parser.py -print", "Get-Location; Get-ChildItem"]:
            self.assertTrue(listing_only("exec_command", {"command": command}))
        for command in ["find . -delete", "ls; python -c 'assert True'", "ls > output", "ls $(touch x)", "cat parser.py"]:
            self.assertFalse(listing_only("exec_command", {"command": command}))

    def test_only_completed_assistant_messages_count(self):
        message = {"role": "assistant", "content": [{"type": "toolCall", "name": "bash", "arguments": {"command": "pwd"}}]}
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "stream.jsonl"
            path.write_text("\n".join(json.dumps({"type": kind, "message": message}) for kind in ["message_start", "message_end"]), encoding="utf-8")
            self.assertEqual(list(calls(path)), [[("bash", {"command": "pwd"})]])

if __name__ == "__main__":
    unittest.main()
