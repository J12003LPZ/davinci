#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
crate = ROOT / "crates/davinci-coding-agent/src/native_extensions"

# Instrument the focused regression without changing its assertions.
path = crate / "ecosystem/mod.rs"
text = path.read_text()

anchor0 = '''        let learning = crate::native_extensions::LearningController::new(dir.path(), None, None);\n        std::fs::create_dir_all(dir.path().join("src")).unwrap();\n'''
replacement0 = '''        let learning = crate::native_extensions::LearningController::new(dir.path(), None, None);\n        let short_candidates = learning.graph_skill_candidates(\n            "shared workflow rust debugging",\n            Role::Writer,\n            2,\n            1000,\n        );\n        eprintln!(\n            "credit-debug short-query-candidates={:?}",\n            short_candidates\n                .iter()\n                .map(|candidate| (candidate.name.clone(), candidate.score))\n                .collect::<Vec<_>>()\n        );\n        std::fs::create_dir_all(dir.path().join("src")).unwrap();\n'''
if text.count(anchor0) != 1:
    raise SystemExit(f"short-candidate anchor expected once, found {text.count(anchor0)}")
text = text.replace(anchor0, replacement0, 1)

anchor = '''        assert_eq!(run.phase, Phase::Done);\n\n        let reloaded = crate::native_extensions::LearningController::new(dir.path(), None, None);\n'''
replacement = '''        assert_eq!(run.phase, Phase::Done);\n        eprintln!("credit-debug verification={:?}", run.verification);\n        for task in &run.tasks {\n            let mutation_files = task\n                .mutation\n                .as_ref()\n                .map(|mutation| mutation.files.iter().map(|file| file.path.clone()).collect::<Vec<_>>())\n                .unwrap_or_default();\n            eprintln!(\n                "credit-debug task={} role={} skills={:?} mutations={:?}",\n                task.id, task.role, task.skill_refs, mutation_files\n            );\n        }\n\n        let reloaded = crate::native_extensions::LearningController::new(dir.path(), None, None);\n'''
if text.count(anchor) != 1:
    raise SystemExit(f"diagnostic anchor expected once, found {text.count(anchor)}")
text = text.replace(anchor, replacement, 1)

anchor2 = '''        let relevant = reloaded.project_store.skill("zzz-relevant").unwrap();\n        let irrelevant = reloaded.project_store.skill("aaa-irrelevant").unwrap();\n        assert_eq!(relevant.success_count, 1);\n'''
replacement2 = '''        let relevant = reloaded.project_store.skill("zzz-relevant").unwrap();\n        let irrelevant = reloaded.project_store.skill("aaa-irrelevant").unwrap();\n        eprintln!(\n            "credit-debug relevant success={} failure={} neutral={}",\n            relevant.success_count, relevant.failure_count, relevant.neutral_count\n        );\n        eprintln!(\n            "credit-debug irrelevant success={} failure={} neutral={}",\n            irrelevant.success_count, irrelevant.failure_count, irrelevant.neutral_count\n        );\n        assert_eq!(relevant.success_count, 1);\n'''
if text.count(anchor2) != 1:
    raise SystemExit(f"diagnostic result anchor expected once, found {text.count(anchor2)}")
path.write_text(text.replace(anchor2, replacement2, 1))

# Instrument the exact query handed from Graph execution to capability selection.
controller = crate / "graph/controller.rs"
text = controller.read_text()
anchor3 = '''                    }\n                    .render();\n                    crate::native_extensions::ecosystem::select_capabilities(\n'''
replacement3 = '''                    }\n                    .render();\n                    let diagnostic_candidates = learn.graph_skill_candidates(\n                        &context_query,\n                        role,\n                        crate::native_extensions::ecosystem::DEFAULT_GRAPH_SKILL_COUNT,\n                        crate::native_extensions::ecosystem::DEFAULT_GRAPH_SKILL_TOKENS,\n                    );\n                    eprintln!(\n                        "capability-debug role={} query_tokens={} query={:?} candidates={:?}",\n                        role,\n                        context_query.split_whitespace().count(),\n                        context_query,\n                        diagnostic_candidates\n                            .iter()\n                            .map(|candidate| (candidate.name.clone(), candidate.score))\n                            .collect::<Vec<_>>()\n                    );\n                    crate::native_extensions::ecosystem::select_capabilities(\n'''
if text.count(anchor3) != 1:
    raise SystemExit(f"controller diagnostic anchor expected once, found {text.count(anchor3)}")
controller.write_text(text.replace(anchor3, replacement3, 1))
