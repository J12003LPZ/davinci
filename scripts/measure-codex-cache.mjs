// Usage: node scripts/measure-codex-cache.mjs <session-or-event-stream.jsonl>
// Legacy normalized snapshots cannot establish that zero-valued fields were measured.
import { readFileSync } from "node:fs";
import { usageFromEvents } from "./subscription-usage.mjs";

const file = process.argv[2];
if (!file) {
  console.error("usage: node scripts/measure-codex-cache.mjs <session-or-event-stream.jsonl>");
  process.exit(2);
}
const lines = readFileSync(file, "utf8").split("\n").map((line) => {
  let entry;
  try { entry = JSON.parse(line); } catch { return line; }
  // Session message entries and streamed terminal messages represent the same
  // normalized snapshot. Neither is a substitute for raw attempt receipts.
  if (entry?.message?.role === "assistant" && entry.type !== "message_update") {
    return JSON.stringify({ type: "message_end", message: entry.message });
  }
  return line;
});
const total = usageFromEvents("davinci", lines);
const display = (value) => value === null ? "unknown" : value;
const ratio = total.readRatio === null ? "n/a" : `${(total.readRatio * 100).toFixed(1)}%`;
console.log(`total source=${total.usageSource} usageRecords=${total.usageRecords} unknownUsageRecords=${total.unknownUsageRecords} fresh=${display(total.input)} read=${display(total.cached)} write=${display(total.cacheWrite)} output=${display(total.output)} readRatio=${ratio}`);
console.log("Observed evidence only; no included-plan allowance or credit estimate. Legacy transcript counters are unverified; missing attempts or other sessions are outside this stream.");
