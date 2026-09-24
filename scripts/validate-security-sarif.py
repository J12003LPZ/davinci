"""Validate native exporter test packets against the vendored OASIS schema offline."""
import json
import pathlib
import sys

from jsonschema import Draft4Validator, FormatChecker
from referencing import Registry
from referencing.exceptions import NoSuchResource


def deny_remote(uri):
    raise NoSuchResource(ref=uri)


schema_path = pathlib.Path(__file__).resolve().parents[1] / "fixtures/sarif/schema.json"
schema = json.loads(schema_path.read_text(encoding="utf-8"))
Draft4Validator.check_schema(schema)
validator = Draft4Validator(schema, format_checker=FormatChecker(), registry=Registry(retrieve=deny_remote))
packets = json.load(sys.stdin)
if not isinstance(packets, list) or not packets:
    raise ValueError("expected nonempty export test packets")
for packet in packets:
    validator.validate(packet)

# Prove that the full schema and URI format checks actually reject bad output.
for invalid in [
    {"version": "2.0.0", "runs": []},
    {"version": "2.1.0", "runs": [{"tool": {"driver": {"name": "fixture"}}, "results": [
        {"message": {"text": "invalid region"}, "locations": [{"physicalLocation": {
            "artifactLocation": {"uri": "src/file.rs"}, "region": {"startLine": 0}}}]}]}]},
    {"version": "2.1.0", "runs": [{"tool": {"driver": {"name": "fixture"}}, "results": [
        {"message": {"text": "invalid URI"}, "locations": [{"physicalLocation": {
            "artifactLocation": {"uri": "src/invalid space.rs"}}}]}]}]},
]:
    if validator.is_valid(invalid):
        raise AssertionError("schema validator accepted invalid sentinel")
print(f"Validated {len(packets)} native SARIF exports offline")
