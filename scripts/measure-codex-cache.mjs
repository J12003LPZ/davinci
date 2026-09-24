// Prints provider-reported cache use for each assistant request in a DaVinci
// session file. Usage: node scripts/measure-codex-cache.mjs <session.jsonl>
import { readFileSync } from "node:fs";

const file = process.argv[2];
if (!file) {
  console.error("usage: node scripts/measure-codex-cache.mjs <session.jsonl>");
  process.exit(2);
}

let totalFresh = 0;
let totalRead = 0;
let totalWrite = 0;
let request = 0;
for (const line of readFileSync(file, "utf8").split("\n")) {
  if (!line.trim()) continue;
  let entry;
  try {
    entry = JSON.parse(line);
  } catch {
    continue;
  }
  const message = entry.message;
  if (!message || message.role !== "assistant" || !message.usage) continue;
  request += 1;
  const fresh = message.usage.input ?? 0;
  const read = message.usage.cacheRead ?? 0;
  const write = message.usage.cacheWrite ?? 0;
  totalFresh += fresh;
  totalRead += read;
  totalWrite += write;
  const raw = fresh + read + write;
  const ratio = raw > 0 ? ((read / raw) * 100).toFixed(1) + "%" : "n/a";
  console.log(
    `#${request} fresh=${fresh} read=${read} write=${write} readRatio=${ratio}`,
  );
}
const raw = totalFresh + totalRead + totalWrite;
console.log(
  `total requests=${request} fresh=${totalFresh} read=${totalRead} write=${totalWrite} readRatio=${
    raw > 0 ? ((totalRead / raw) * 100).toFixed(1) + "%" : "n/a"
  }`,
);
