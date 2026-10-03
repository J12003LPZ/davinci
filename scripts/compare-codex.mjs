// Runs the same tasks in Codex CLI and DaVinci on the same model and prints
// tokens, wall time and test results. A person runs it; it spends plan credits.
// Usage: node scripts/compare-codex.mjs <fixture-repo> <model> <tasks.json>
import { spawnSync } from "node:child_process";
import { cpSync, mkdtempSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";
import { join } from "node:path";

const [fixture, model, tasksFile] = process.argv.slice(2);
if (!fixture || !model || !tasksFile) {
  console.error(
    "usage: node scripts/compare-codex.mjs <fixture-repo> <model> <tasks.json>",
  );
  process.exit(2);
}
const tasks = JSON.parse(readFileSync(tasksFile, "utf8"));
const runsPerHarness = Number.parseInt(process.env.DAVINCI_COMPARE_RUNS ?? "3", 10);
if (!Number.isInteger(runsPerHarness) || runsPerHarness < 1) {
  throw new Error("DAVINCI_COMPARE_RUNS must be a positive integer");
}

const catalogPath = fileURLToPath(
  new URL("../crates/davinci-ai/src/catalogs.json", import.meta.url),
);
const catalog = JSON.parse(readFileSync(catalogPath, "utf8"));

function findModelCost(value) {
  if (!value || typeof value !== "object") return null;
  if (
    !Array.isArray(value) &&
    value.provider === "openai-codex" &&
    value.id === model &&
    value.cost
  ) {
    return value.cost;
  }
  for (const child of Object.values(value)) {
    const found = findModelCost(child);
    if (found) return found;
  }
  return null;
}

const modelCost = findModelCost(catalog);
if (!modelCost) {
  throw new Error(`no openai-codex catalog cost found for ${model}`);
}

function creditsFor(usage) {
  const usd =
    ((usage.input ?? 0) / 1_000_000) * (modelCost.input ?? 0) +
    ((usage.cached ?? 0) / 1_000_000) * (modelCost.cacheRead ?? modelCost.cache_read ?? 0) +
    ((usage.output ?? 0) / 1_000_000) * (modelCost.output ?? 0);
  return usd * 25;
}

function freshCopy() {
  const dir = mkdtempSync(join(tmpdir(), "davinci-codex-cmp-"));
  cpSync(fixture, dir, { recursive: true });
  return dir;
}

function sumUsage(lines, pick) {
  const total = { input: 0, cached: 0, output: 0 };
  for (const line of lines) {
    let event;
    try {
      event = JSON.parse(line);
    } catch {
      continue;
    }
    const usage = pick(event);
    if (!usage) continue;
    total.input += usage.input ?? 0;
    total.cached += usage.cached ?? 0;
    total.output += usage.output ?? 0;
  }
  return total;
}

function run(harness, task) {
  const cwd = freshCopy();
  const started = Date.now();
  const result =
    harness === "codex"
      ? spawnSync("codex", ["exec", "--json", "-m", model, task.prompt], {
          cwd,
          encoding: "utf8",
          shell: true,
        })
      : spawnSync(
          "davinci",
          [
            "--mode",
            "json",
            "--model",
            `openai-codex/${model}`,
            "--permission-mode",
            "auto",
            "-p",
            task.prompt,
          ],
          { cwd, encoding: "utf8", shell: true },
        );
  const wallMs = Date.now() - started;
  const lines = (result.stdout ?? "").split("\n");
  const usage =
    harness === "codex"
      ? sumUsage(lines, (event) =>
          event.type === "turn.completed" && event.usage
            ? {
                input:
                  event.usage.input_tokens -
                  (event.usage.cached_input_tokens ?? 0),
                cached: event.usage.cached_input_tokens,
                output: event.usage.output_tokens,
              }
            : null,
        )
      : sumUsage(lines, (event) =>
          event.type === "message_end" &&
          event.message?.role === "assistant" &&
          event.message.usage
            ? {
                input: event.message.usage.input,
                cached: event.message.usage.cacheRead,
                output: event.message.usage.output,
              }
            : null,
        );
  const check = spawnSync(task.check, {
    cwd,
    encoding: "utf8",
    shell: true,
  });
  return {
    harness,
    task: task.name,
    wallMs,
    ...usage,
    credits: creditsFor(usage),
    passed: check.status === 0,
    exitCode: result.status,
  };
}

const rows = [];
for (const task of tasks) {
  for (const harness of ["codex", "davinci"]) {
    for (let runIndex = 1; runIndex <= runsPerHarness; runIndex += 1) {
      const row = { run: runIndex, ...run(harness, task) };
      console.log(JSON.stringify(row));
      rows.push(row);
    }
  }
}
console.table(rows);

const groups = new Map();
for (const row of rows) {
  const key = `${row.harness}\0${row.task}`;
  const group = groups.get(key) ?? [];
  group.push(row);
  groups.set(key, group);
}
const means = [...groups.entries()].map(([key, group]) => {
  const [harness, task] = key.split("\0");
  const mean = (field) =>
    group.reduce((sum, row) => sum + Number(row[field] ?? 0), 0) / group.length;
  return {
    harness,
    task,
    runs: group.length,
    input: mean("input"),
    cached: mean("cached"),
    output: mean("output"),
    credits: mean("credits"),
    wallMs: mean("wallMs"),
    passRate: group.filter((row) => row.passed).length / group.length,
  };
});
console.table(means);
