#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
path = ROOT / "crates/davinci-coding-agent/src/native_extensions/graph/controller.rs"
text = path.read_text()

old = r'''        let mut seen = std::collections::HashSet::new();
        for task in &run.tasks {
            for s in &task.skill_refs {
                let key = (s.name.clone(), s.version, s.content_hash.clone());
                if seen.insert(key) {
                    let version_ref = crate::native_extensions::learning::types::SkillVersionRef {
                        name: s.name.clone(),
                        version: s.version,
                        content_hash: s.content_hash.clone(),
                    };
                    let exact_record = learning
                        .project_store
                        .skill_version(&s.name, s.version)
                        .or_else(|| learning.global_store.skill_version(&s.name, s.version))
                        .filter(|record| record.content_hash == s.content_hash)
                        .cloned();
                    let relevant = exact_record
                        .as_ref()
                        .is_some_and(|record| Self::skill_scope_relevant(record, task, &run.goal));
                    let signal = Self::skill_usage_signal(outcome, relevant);
                    let _ = learning.record_skill_usage_outcome(&version_ref, signal);
                }
            }
        }
'''

new = r'''        let mut usage = std::collections::BTreeMap::new();
        for task in &run.tasks {
            for s in &task.skill_refs {
                let key = (s.name.clone(), s.version, s.content_hash.clone());
                let exact_record = learning
                    .project_store
                    .skill_version(&s.name, s.version)
                    .or_else(|| learning.global_store.skill_version(&s.name, s.version))
                    .filter(|record| record.content_hash == s.content_hash)
                    .cloned();
                let relevant = exact_record
                    .as_ref()
                    .is_some_and(|record| Self::skill_scope_relevant(record, task, &run.goal));
                usage
                    .entry(key)
                    .and_modify(|seen_relevant| *seen_relevant |= relevant)
                    .or_insert(relevant);
            }
        }
        for ((name, version, content_hash), relevant) in usage {
            let version_ref = crate::native_extensions::learning::types::SkillVersionRef {
                name,
                version,
                content_hash,
            };
            let signal = Self::skill_usage_signal(outcome, relevant);
            let _ = learning.record_skill_usage_outcome(&version_ref, signal);
        }
'''

count = text.count(old)
if count != 1:
    raise SystemExit(f"expected exactly one generated skill outcome loop, found {count}")
path.write_text(text.replace(old, new, 1))
