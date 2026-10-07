"""A stand-in for `codex app-server` in the offline evals.

DaVinci reads ChatGPT plan usage from the Codex app-server over stdio
JSON-RPC. This answers `initialize` and `account/rateLimits/read` with a Plus
plan (5-hour window 27% used, weekly 59% used), then sends one sparse
`account/rateLimits/updated` notification that moves only the 5-hour window
to 31%, in the slot the read used for the weekly one. The screen should end
on 69% left for 5h and 41% left for the week: the read was parsed, the
windows were told apart by length, and the update was merged without
dropping the weekly window.
"""
import json
import sys
import time

if len(sys.argv) < 2 or sys.argv[1] != "app-server":
    sys.exit(2)


def send(message):
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


now = int(time.time())
for line in sys.stdin:
    try:
        message = json.loads(line)
    except ValueError:
        continue
    method = message.get("method")
    if method == "initialize":
        send({"id": message["id"], "result": {"userAgent": "fake-codex/0"}})
    elif method == "account/rateLimits/read":
        send({"id": message["id"], "result": {"rateLimits": {
            # Weekly in the primary slot on purpose: slots carry no meaning.
            "primary": {"usedPercent": 59, "windowDurationMins": 10080, "resetsAt": now + 3 * 86400 + 4 * 3600 + 120},
            "secondary": {"usedPercent": 27, "windowDurationMins": 300, "resetsAt": now + 2 * 3600 + 14 * 60 + 30},
            "planType": "plus",
        }}})
        time.sleep(0.5)
        send({"method": "account/rateLimits/updated", "params": {"rateLimits": {
            # The other slot: it must still replace the 5-hour window.
            "primary": {"usedPercent": 31, "windowDurationMins": 300},
        }}})
