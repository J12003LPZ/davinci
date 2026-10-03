// Prints provider-reported cache use for each assistant request in a DaVinci
// session file. Usage: node scripts/measure-codex-cache.mjs <session.jsonl>
import { readFileSync } from "node:fs";
import { davinciUsage, summarizeUsage } from "./subscription-usage.mjs";

const file = process.argv[2];
if (!file) {
  console.error("usage: node scripts/measure-codex-cache.mjs <session.jsonl>");
  process.exit(2);
}

const records = [];
const display = (value) => value === null ? "unknown" : value;
const ratio = (value) => value === null ? "n/a" : `${(value * 100).toFixed(1)}%`;
for (const line of readFileSync(file, "utf8").split("\n")) {
  if (!line.trim()) continue;
  let entry;
  try {
    entry = JSON.parse(line);
  } catch {
    continue;
  }
  const message = entry?.message;
  if (!message || message.role !== "assistant") continue;
  const record = davinciUsage(message.usage);
  records.push(record);
  const summary = summarizeUsage([record]);
  console.log(
    `#${records.length} fresh=${display(record.input)} read=${display(record.cached)} write=${display(record.cacheWrite)} readRatio=${ratio(summary.readRatio)}`,
  );
}
const total = summarizeUsage(records);
console.log(
  `total usageRecords=${total.usageRecords} unknownUsageRecords=${total.unknownUsageRecords} fresh=${display(total.input)} read=${display(total.cached)} write=${display(total.cacheWrite)} readRatio=${ratio(total.readRatio)}`,
);
console.log("Observed transcript tokens; no included-plan allowance or credit estimate. Retries and side calls may not appear in this transcript.");
