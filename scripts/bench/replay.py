"""Replay recorded responses through the Rust agent, without a live provider.

Only public task prompts and recorded assistant responses reach the executable.
The hidden grader and reference solution are never loaded by this runner.
"""
import argparse
import json
import os
from pathlib import Path
import re

import bench
from campaign import file_hash
from runner import controlled_environment, execute


def remap(value, old, new):
    if isinstance(value, str):
        # Recorded Windows shell arguments use either slash convention. Change
        # only the exact original workdir prefix, including a path boundary.
        result = value
        for source, target in ((old.replace("\\", "/"), new.replace("\\", "/")),
                               (old.replace("/", "\\"), new.replace("/", "\\"))):
            result = re.sub(re.escape(source) + r"(?=$|[\\/\s'\"])",
                            lambda _: target, result)
        return result
    if isinstance(value, list):
        return [remap(item, old, new) for item in value]
    if isinstance(value, dict):
        return {key: remap(item, old, new) for key, item in value.items()}
    return value


def recording(stdout, original_workdir, fresh_workdir):
    frames, reminders, ids, results = [], [], set(), []
    for line in stdout.splitlines():
        if not line.lstrip().startswith("{"):
            continue
        event = json.loads(line)
        if not isinstance(event, dict):
            continue
        if event.get("type") == "tool_execution_end":
            if not isinstance(event.get("toolCallId"), str) or type(event.get("isError")) is not bool:
                raise ValueError("invalid recorded tool outcome")
            results.append({"id": event["toolCallId"], "is_error": event["isError"],
                            "result": event.get("result")})
        if event.get("type") == "message_end":
            message = event.get("message", {})
            if isinstance(message, dict):
                reason = message.get("davinciCapabilityReminder")
                if isinstance(reason, str):
                    reminders.append(reason)
        update = event.get("assistantMessageEvent")
        if event.get("type") != "message_update" or not isinstance(update, dict) or update.get("type") != "done":
            continue
        message = update.get("message")
        if not isinstance(message, dict) or message.get("role") != "assistant":
            raise ValueError("invalid recorded assistant response")
        identity = message.get("id")
        if not isinstance(identity, str) or not identity or identity in ids:
            raise ValueError("missing or duplicate recorded response id")
        ids.add(identity)
        content = message.get("content")
        if not isinstance(content, list):
            raise ValueError("invalid recorded content")
        transformed = [dict(block, arguments=remap(block.get("arguments"), original_workdir, fresh_workdir))
                       if isinstance(block, dict) and block.get("type") == "toolCall" else block
                       for block in content]
        frames.append({"assistant": dict(message, content=transformed),
                       "reminders_before": list(reminders), "results_before": list(results)})
    if not frames:
        raise ValueError("no recorded model responses")
    return frames


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--recordings", type=Path, required=True)
    parser.add_argument("--original-runs", type=Path, required=True)
    parser.add_argument("--runs", type=Path, required=True)
    parser.add_argument("--replay-binary", type=Path, required=True)
    args = parser.parse_args()
    root = args.runs.resolve()
    root.mkdir(parents=True, exist_ok=False)
    binary = args.replay_binary.resolve()
    identity = file_hash(binary)
    streams = sorted((args.recordings / "davinci").glob("*.stdout.jsonl"))
    if not streams:
        raise ValueError("no DaVinci recordings found")
    rows = []
    for stream in streams:
        name = stream.name.removesuffix(".stdout.jsonl")
        match = re.fullmatch(r"(.+)-r(\d+)", name)
        if not match or match[1] not in bench.task_ids():
            raise ValueError("recording is outside the frozen legacy tasks")
        tid, rep = match[1], int(match[2])
        workdir = root / name
        bench.prepare(tid, str(workdir))
        frames = recording(stream.read_text(encoding="utf-8"),
                           str(args.original_runs / "davinci" / name), str(workdir))
        public_input = root / (name + ".replay.json")
        public_input.write_text(json.dumps({"prompt": bench.load(tid)["prompt"], "frames": frames}), encoding="utf-8")
        measured = execute([str(binary), str(public_input), str(workdir)], workdir,
                           controlled_environment(os.environ, root / "_empty_agent"), 180)
        if measured["exit"] != 0:
            raise RuntimeError("offline replay process failed: " + name + ": " + str(measured["exit"]))
        summary = json.loads(measured["stdout"].splitlines()[-1])
        row = dict(summary, task=tid, rep=rep, binary_sha256=identity,
                   recording_sha256=file_hash(stream))
        rows.append(row)
        print(json.dumps(row), flush=True)
        with (root / "results.jsonl").open("a", encoding="utf-8") as output:
            output.write(json.dumps(row) + "\n")
    (root / "summary.json").write_text(json.dumps({"runs": len(rows),
        "provided_requests": sum(r["provided_requests"] for r in rows),
        "requested_requests": sum(r["requested_requests"] for r in rows),
        "diverged_runs": sum(r["stop_reason"] is not None for r in rows)}, indent=2), encoding="utf-8")


if __name__ == "__main__":
    main()
