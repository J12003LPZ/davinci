// Runs the same tasks in Codex CLI and DaVinci on the same model and prints
// observed tokens, wall time and test results. Running it consumes subscription usage.
// Usage: node scripts/compare-codex.mjs <fixture-repo> <model> <tasks.json>
import { spawnSync } from "node:child_process";
import { cpSync, mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { usageFromEvents, summarizeRuns } from "./subscription-usage.mjs";

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

function freshCopy() {
  const dir = mkdtempSync(join(tmpdir(), "davinci-codex-cmp-"));
  cpSync(fixture, dir, { recursive: true });
  return dir;
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
  const usage = usageFromEvents(harness, lines);
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
    passed: result.status === 0 && !result.error && check.status === 0 && !check.error,
    exitCode: result.status,
    checkExitCode: check.status,
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

console.table(summarizeRuns(rows));
console.log("Token observations are not included-plan allowance or credit estimates. Missing usage is null. These CLI events may omit retries and side calls; use the full benchmark runner for acceptance evidence.");
