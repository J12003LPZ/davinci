#!/usr/bin/env node

// One-time migration for external copies of the scenario corpus. The loader
// supplies the same safe default for mutating Core-200 scenarios at runtime.
const fs = require("node:fs");

const path = process.argv[2];
if (!path) {
  console.error("usage: migrate-verification-commands.js <scenario-json>");
  process.exit(2);
}

const scenarios = JSON.parse(fs.readFileSync(path, "utf8"));
for (const scenario of scenarios) {
  scenario.setup_commands ??= [];
  scenario.verification_commands ??= [];
  const mutating = (scenario.requirements ?? []).some(
    (requirement) => requirement.type === "file_changed",
  );
  if (mutating && scenario.verification_commands.length === 0) {
    scenario.verification_commands.push({
      command: "git diff --check",
      timeout_seconds: 30,
      expected_exit: 0,
    });
  }
}
fs.writeFileSync(path, `${JSON.stringify(scenarios, null, 2)}\n`);
