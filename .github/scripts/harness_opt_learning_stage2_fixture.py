#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
path = ROOT / "crates/davinci-coding-agent/src/native_extensions/ecosystem/mod.rs"
text = path.read_text()

old = '''        let learning = crate::native_extensions::LearningController::new(dir.path(), None, None);\n        let runner: Arc<WorkerRunner> = Arc::new(|spec, _abort, _on_progress| {\n            let artifact = match spec.expect {\n'''
new = '''        let learning = crate::native_extensions::LearningController::new(dir.path(), None, None);\n        std::fs::create_dir_all(dir.path().join("src")).unwrap();\n        std::fs::write(dir.path().join("src/auth.rs"), "before").unwrap();\n        let mutation_root = dir.path().to_path_buf();\n        let runner: Arc<WorkerRunner> = Arc::new(move |spec, _abort, _on_progress| {\n            if matches!(spec.expect, ArtifactKind::PatchReport) {\n                std::fs::write(mutation_root.join("src/auth.rs"), "after").unwrap();\n            }\n            let artifact = match spec.expect {\n'''

count = text.count(old)
if count != 1:
    raise SystemExit(f"expected exactly one learning credit fixture anchor, found {count}")
path.write_text(text.replace(old, new, 1))
