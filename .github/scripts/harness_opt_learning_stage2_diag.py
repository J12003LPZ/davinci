#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
path = ROOT / "crates/davinci-coding-agent/src/native_extensions/ecosystem/mod.rs"
text = path.read_text()

anchor = '''        assert_eq!(run.phase, Phase::Done);\n\n        let reloaded = crate::native_extensions::LearningController::new(dir.path(), None, None);\n'''
replacement = '''        assert_eq!(run.phase, Phase::Done);\n        eprintln!("credit-debug verification={:?}", run.verification);\n        for task in &run.tasks {\n            let mutation_files = task\n                .mutation\n                .as_ref()\n                .map(|mutation| mutation.files.iter().map(|file| file.path.clone()).collect::<Vec<_>>())\n                .unwrap_or_default();\n            eprintln!(\n                "credit-debug task={} role={} skills={:?} mutations={:?}",\n                task.id, task.role, task.skill_refs, mutation_files\n            );\n        }\n\n        let reloaded = crate::native_extensions::LearningController::new(dir.path(), None, None);\n'''
if text.count(anchor) != 1:
    raise SystemExit(f"diagnostic anchor expected once, found {text.count(anchor)}")
text = text.replace(anchor, replacement, 1)

anchor2 = '''        let relevant = reloaded.project_store.skill("zzz-relevant").unwrap();\n        let irrelevant = reloaded.project_store.skill("aaa-irrelevant").unwrap();\n        assert_eq!(relevant.success_count, 1);\n'''
replacement2 = '''        let relevant = reloaded.project_store.skill("zzz-relevant").unwrap();\n        let irrelevant = reloaded.project_store.skill("aaa-irrelevant").unwrap();\n        eprintln!(\n            "credit-debug relevant success={} failure={} neutral={} applicability={:?}",\n            relevant.success_count, relevant.failure_count, relevant.neutral_count, relevant.applicability\n        );\n        eprintln!(\n            "credit-debug irrelevant success={} failure={} neutral={} applicability={:?}",\n            irrelevant.success_count, irrelevant.failure_count, irrelevant.neutral_count, irrelevant.applicability\n        );\n        assert_eq!(relevant.success_count, 1);\n'''
if text.count(anchor2) != 1:
    raise SystemExit(f"diagnostic result anchor expected once, found {text.count(anchor2)}")
path.write_text(text.replace(anchor2, replacement2, 1))
