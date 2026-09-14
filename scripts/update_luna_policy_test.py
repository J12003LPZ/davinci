from pathlib import Path

p = Path("crates/davinci-agent/src/runtime/capabilities.rs")
text = p.read_text(encoding="utf-8")
replacements = [
    (
        '''                "read",
                ConcurrencyPolicy::ParallelSafe,
                ReplayPolicy::SafeToReplay,
                OutputPolicy::Compressible,
''',
        '''                "read",
                ConcurrencyPolicy::ParallelSafe,
                ReplayPolicy::SafeToReplay,
                OutputPolicy::LosslessRequired,
''',
    ),
    (
        '''                "edit",
                ConcurrencyPolicy::SerialBarrier,
                ReplayPolicy::ReconcileBeforeReplay,
                OutputPolicy::Normal,
''',
        '''                "edit",
                ConcurrencyPolicy::SerialBarrier,
                ReplayPolicy::ReconcileBeforeReplay,
                OutputPolicy::LosslessRequired,
''',
    ),
    (
        '''                "exec_command",
                ConcurrencyPolicy::SerialBarrier,
                ReplayPolicy::NeverAutoReplay,
                OutputPolicy::LosslessRequired,
''',
        '''                "exec_command",
                ConcurrencyPolicy::SerialBarrier,
                ReplayPolicy::NeverAutoReplay,
                OutputPolicy::Compressible,
''',
    ),
    (
        '''                "agent",
                ConcurrencyPolicy::ParallelSafe,
                ReplayPolicy::NeverAutoReplay,
                OutputPolicy::Normal,
''',
        '''                "agent",
                ConcurrencyPolicy::ParallelSafe,
                ReplayPolicy::NeverAutoReplay,
                OutputPolicy::LosslessRequired,
''',
    ),
]
for old, new in replacements:
    if old not in text:
        raise SystemExit("expected existing output-policy assertion not found")
    text = text.replace(old, new, 1)
p.write_text(text, encoding="utf-8")
