#!/usr/bin/env python3
from __future__ import annotations

import argparse
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one match, found {count}")
    return text.replace(old, new, 1)


def add_tests() -> None:
    ledger_path = ROOT / "crates/davinci-agent/src/tool_ledger.rs"
    ledger = ledger_path.read_text()
    if "fn tool_ledger_atomic_persist_replaces_complete_state()" not in ledger:
        idx = ledger.rfind("\n}")
        test = r'''

    #[test]
    fn tool_ledger_atomic_persist_replaces_complete_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ledger.json");
        atomic_write_json(&path, br#"{"generation":1}"#).unwrap();
        atomic_write_json(&path, br#"{"generation":2,"complete":true}"#).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["generation"], 2);
        assert_eq!(value["complete"], true);
        assert!(std::fs::read_dir(dir.path()).unwrap().all(|entry| {
            !entry.unwrap().file_name().to_string_lossy().contains(".tmp-")
        }));
    }

    #[test]
    fn corrupt_tool_ledger_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tool-ledger.json");
        std::fs::write(&path, b"{not-json").unwrap();
        let error = ToolCallLedger::load_bound(&path, "session-a").unwrap_err();
        assert!(error.contains("tool ledger is corrupt"));
    }
'''
        ledger = ledger[:idx] + test + ledger[idx:]
        ledger_path.write_text(ledger)

    turn_path = ROOT / "crates/davinci-agent/src/turn.rs"
    turn = turn_path.read_text()
    if "fn tool_ledger_uses_runtime_capability_side_effect()" not in turn:
        idx = turn.rfind("\n}")
        test = r'''

    #[test]
    fn tool_ledger_uses_runtime_capability_side_effect() {
        let mut agent = Agent::new("x");
        let runtime = crate::RuntimeHandle::new(
            crate::RunId::new(),
            crate::AgentId::new(),
            crate::RuntimeBus::new(),
        );
        runtime.capability_registry.register(crate::RuntimeCapability::new(
            "custom_read_capability",
            crate::CapabilitySource::Mcp,
            crate::ToolClass::Read,
            true,
            &serde_json::json!({"type":"object"}),
            None,
        ));
        agent.set_runtime(runtime);

        let side_effect = agent.side_effect_for_tool("custom_read_capability");
        assert_eq!(side_effect, crate::tool_ledger::ToolSideEffect::ReadOnly);

        let mut ledger = agent.tool_ledger.lock().unwrap();
        ledger
            .reserve_call_with_metadata(
                "custom-read-call",
                "custom_read_capability",
                &serde_json::json!({}),
                crate::runtime::ReplayPolicy::SafeToReplay,
                side_effect,
            )
            .unwrap();
        let record = ledger.records().get("custom-read-call").unwrap();
        assert_eq!(record.side_effect, crate::tool_ledger::ToolSideEffect::ReadOnly);
    }
'''
        turn_path.write_text(turn[:idx] + test + turn[idx:])


def apply_impl() -> None:
    ledger_path = ROOT / "crates/davinci-agent/src/tool_ledger.rs"
    ledger = ledger_path.read_text()
    ledger = replace_once(
        ledger,
        "use std::collections::HashMap;\nuse std::path::{Path, PathBuf};\n",
        "use std::collections::HashMap;\nuse std::io::Write;\nuse std::path::{Path, PathBuf};\n",
        "ledger imports",
    )
    helper = r'''

fn atomic_write_json(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "tool ledger path has no parent directory".to_string())?;
    std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    let name = path.file_name().and_then(|value| value.to_str()).unwrap_or("ledger");
    let temp = parent.join(format!(".{name}.tmp-{}-{}", std::process::id(), now_millis()));
    let write_result = (|| -> Result<(), String> {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .map_err(|err| err.to_string())?;
        file.write_all(bytes).map_err(|err| err.to_string())?;
        file.sync_all().map_err(|err| err.to_string())?;

        #[cfg(windows)]
        if path.exists() {
            let previous = parent.join(format!(".{name}.previous"));
            let _ = std::fs::remove_file(&previous);
            std::fs::rename(path, &previous).map_err(|err| err.to_string())?;
            if let Err(error) = std::fs::rename(&temp, path) {
                let _ = std::fs::rename(&previous, path);
                return Err(error.to_string());
            }
            let _ = std::fs::remove_file(previous);
        }
        #[cfg(not(windows))]
        std::fs::rename(&temp, path).map_err(|err| err.to_string())?;
        #[cfg(windows)]
        if !path.exists() {
            std::fs::rename(&temp, path).map_err(|err| err.to_string())?;
        }

        if let Ok(directory) = std::fs::File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    write_result
}

fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
'''
    marker = "\nfn default_condvar() -> Arc<Condvar> {"
    if "fn atomic_write_json(" not in ledger:
        ledger = replace_once(ledger, marker, helper + marker, "atomic helper insertion")

    old_persist = r'''    pub fn persist(&self) -> Result<(), String> {
        let Some(path) = &self.persistence_path else {
            return Ok(());
        };
        let bytes = serde_json::to_vec_pretty(self).map_err(|err| err.to_string())?;
        std::fs::write(path, bytes).map_err(|err| err.to_string())?;
        std::fs::File::open(path)
            .and_then(|file| file.sync_all())
            .map_err(|err| err.to_string())
    }
'''
    new_persist = r'''    pub fn persist(&self) -> Result<(), String> {
        let Some(path) = &self.persistence_path else {
            return Ok(());
        };
        let bytes = serde_json::to_vec_pretty(self).map_err(|err| err.to_string())?;
        atomic_write_json(path, &bytes)
    }
'''
    ledger = replace_once(ledger, old_persist, new_persist, "atomic ledger persist")

    old_sig = r'''    pub fn reserve_call_with_policy(
        &mut self,
        call_id: &str,
        tool_name: &str,
        arguments: &Value,
        replay_policy: ReplayPolicy,
    ) -> Result<ReservationOutcome, String> {
        let norm_args = normalize_arguments(arguments);
'''
    new_sig = r'''    pub fn reserve_call_with_policy(
        &mut self,
        call_id: &str,
        tool_name: &str,
        arguments: &Value,
        replay_policy: ReplayPolicy,
    ) -> Result<ReservationOutcome, String> {
        self.reserve_call_with_metadata(
            call_id,
            tool_name,
            arguments,
            replay_policy,
            classify_side_effect(tool_name),
        )
    }

    pub fn reserve_call_with_metadata(
        &mut self,
        call_id: &str,
        tool_name: &str,
        arguments: &Value,
        replay_policy: ReplayPolicy,
        side_effect: ToolSideEffect,
    ) -> Result<ReservationOutcome, String> {
        let norm_args = normalize_arguments(arguments);
'''
    ledger = replace_once(ledger, old_sig, new_sig, "metadata reservation signature")
    ledger = replace_once(
        ledger,
        "                side_effect: classify_side_effect(tool_name),\n                replay_policy,\n",
        "                side_effect,\n                replay_policy,\n",
        "reserved side effect metadata",
    )
    ledger_path.write_text(ledger)

    turn_path = ROOT / "crates/davinci-agent/src/turn.rs"
    turn = turn_path.read_text()
    replay_method = r'''    fn replay_policy_for_tool(&self, name: &str) -> crate::runtime::ReplayPolicy {
        self.runtime
            .as_ref()
            .and_then(|runtime| runtime.capability_registry.get(name))
            .map(|capability| capability.replay_policy)
            .unwrap_or_else(|| crate::runtime::conservative_replay_policy(name))
    }
'''
    side_effect_method = replay_method + r'''

    fn side_effect_for_tool(&self, name: &str) -> crate::tool_ledger::ToolSideEffect {
        self.runtime
            .as_ref()
            .and_then(|runtime| runtime.capability_registry.get(name))
            .map(|capability| {
                if capability.read_only {
                    crate::tool_ledger::ToolSideEffect::ReadOnly
                } else {
                    crate::tool_ledger::ToolSideEffect::Mutating
                }
            })
            .unwrap_or_else(|| crate::tool_ledger::classify_side_effect(name))
    }
'''
    turn = replace_once(turn, replay_method, side_effect_method, "runtime side effect method")
    turn = replace_once(
        turn,
        '''        let replay_policy = self.replay_policy_for_tool(name);
        if let Ok(mut ledger) = self.tool_ledger.lock() {
            match ledger.reserve_call_with_policy(id, name, args, replay_policy) {
''',
        '''        let replay_policy = self.replay_policy_for_tool(name);
        let side_effect = self.side_effect_for_tool(name);
        if let Ok(mut ledger) = self.tool_ledger.lock() {
            match ledger.reserve_call_with_metadata(id, name, args, replay_policy, side_effect) {
''',
        "live reservation metadata",
    )
    turn_path.write_text(turn)


def main() -> None:
    parser = argparse.ArgumentParser()
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--tests", action="store_true")
    mode.add_argument("--impl", action="store_true")
    args = parser.parse_args()
    if args.tests:
        add_tests()
    else:
        apply_impl()


if __name__ == "__main__":
    main()
