//! Reproducible local evaluation; no model calls, paid services, or timing assertions.
use davinci_coding_agent::native_extensions::repo_intelligence::{
    RepoIntelligence, RepoIntelligenceConfig,
};
use serde_json::{json, Value};
use std::{fs, path::Path, time::Instant};

fn elapsed(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

fn fixture(root: &Path) -> usize {
    fs::create_dir_all(root.join("src/api")).unwrap();
    fs::create_dir(root.join("tests")).unwrap();
    let files = [
        ("src/auth.ts", "export class AuthService { login() { return true; } }"),
        ("src/session.ts", "import {AuthService} from './auth'; export function validateSession() { return new AuthService().login(); }"),
        ("src/api/router.ts", "import {validateSession} from '../session'; export function router() { return validateSession(); }"),
        ("src/api/index.ts", "export {router} from './router';"),
        ("tests/session.test.ts", "import {validateSession} from '../src/session'; export function testSession() { return validateSession(); }"),
        ("tsconfig.json", "{\"compilerOptions\":{\"strict\":true}}"),
    ];
    let mut bytes = 0;
    for (path, body) in files {
        // Ordinary unrelated implementation text makes whole-file reads measurable.
        let padding = if path.ends_with(".ts") {
            "// unrelated implementation detail\n".repeat(180)
        } else {
            String::new()
        };
        let body = format!("{body}\n{padding}");
        bytes += body.len();
        fs::write(root.join(path), body).unwrap();
    }
    for n in 0..195 {
        let body = format!("export function unrelated{n}() {{ return {n}; }}\n");
        bytes += body.len();
        fs::write(root.join(format!("src/module{n}.ts")), body).unwrap();
    }
    bytes
}

fn baseline(root: &Path, pattern: &str, reads: &[&str]) -> (Value, String) {
    let start = Instant::now();
    let grep = davinci_agent::tools::execute_tool(
        root,
        "grep",
        &json!({"pattern":pattern,"path":".","limit":100}),
    )
    .unwrap();
    assert!(!grep.is_error);
    let mut content = grep.content;
    let mut read_bytes = 0;
    for path in reads {
        let result =
            davinci_agent::tools::execute_tool(root, "read", &json!({"path":path})).unwrap();
        assert!(!result.is_error);
        read_bytes += fs::metadata(root.join(path)).unwrap().len();
        content.push_str(&result.content);
    }
    (
        json!({"tool_calls":1 + reads.len(),"explicit_read_bytes":read_bytes,
        "returned_bytes":content.len(),"estimated_tokens":content.len().div_ceil(4),
        "latency_ms":elapsed(start),"grep_disk_bytes":"not instrumented"}),
        content,
    )
}

fn indexed(manager: &RepoIntelligence, calls: &[(&str, Value)]) -> (Value, String) {
    let start = Instant::now();
    let mut content = String::new();
    for (name, args) in calls {
        content.push_str(&manager.query(name, args).unwrap().to_string());
    }
    (
        json!({"tool_calls":calls.len(),"explicit_read_bytes":0,"returned_bytes":content.len(),
        "estimated_tokens":content.len().div_ceil(4),"latency_ms":elapsed(start)}),
        content,
    )
}

#[cfg(windows)]
fn peak_memory_bytes() -> Option<usize> {
    #[repr(C)]
    struct Counters {
        size: u32,
        faults: u32,
        peak_working: usize,
        working: usize,
        peak_paged: usize,
        paged: usize,
        peak_nonpaged: usize,
        nonpaged: usize,
        pagefile: usize,
        peak_pagefile: usize,
    }
    #[link(name = "psapi")]
    extern "system" {
        fn GetProcessMemoryInfo(process: isize, counters: *mut Counters, size: u32) -> i32;
    }
    let mut counters = Counters {
        size: std::mem::size_of::<Counters>() as u32,
        faults: 0,
        peak_working: 0,
        working: 0,
        peak_paged: 0,
        paged: 0,
        peak_nonpaged: 0,
        nonpaged: 0,
        pagefile: 0,
        peak_pagefile: 0,
    };
    // SAFETY: -1 is the current-process pseudo handle and counters is a correctly sized writable C struct.
    (unsafe { GetProcessMemoryInfo(-1, &mut counters, std::mem::size_of::<Counters>() as u32) }
        != 0)
        .then_some(counters.peak_working)
}

#[cfg(not(windows))]
fn peak_memory_bytes() -> Option<usize> {
    fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|line| {
            line.strip_prefix("VmHWM:")?
                .split_whitespace()
                .next()?
                .parse::<usize>()
                .ok()
                .map(|kb| kb * 1024)
        })
}

#[test]
#[ignore = "explicit measured evaluation; run with --ignored --nocapture"]
fn repo_intelligence_measured_evaluation() {
    let repo = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let source_bytes = fixture(repo.path());
    let start = Instant::now();
    for _ in 0..100 {
        let manager = RepoIntelligence::new(
            repo.path(),
            cache.path(),
            RepoIntelligenceConfig {
                enabled: false,
                ..Default::default()
            },
        );
        assert_eq!(manager.status()["initialized"], false);
    }
    let disabled_startup_ms = elapsed(start) / 100.0;
    let start = Instant::now();
    for _ in 0..100 {
        let manager = RepoIntelligence::new(repo.path(), cache.path(), Default::default());
        assert_eq!(manager.status()["initialized"], false);
    }
    let unused_startup_ms = elapsed(start) / 100.0;
    assert!(!cache.path().join("repo-index").exists());
    let manager = RepoIntelligence::new(repo.path(), cache.path(), Default::default());
    let start = Instant::now();
    let cold = manager.refresh().unwrap();
    let cold_ms = elapsed(start);
    assert_eq!(cold.reparsed, 200);
    drop(manager);
    let manager = RepoIntelligence::new(repo.path(), cache.path(), Default::default());
    let start = Instant::now();
    let warm = manager.refresh().unwrap();
    let warm_ms = elapsed(start);
    assert_eq!(warm.reparsed, 0);
    fs::write(
        repo.path().join("src/module1.ts"),
        "export function unrelated1() { return 2; }\n",
    )
    .unwrap();
    let start = Instant::now();
    let incremental = manager.refresh().unwrap();
    let incremental_ms = elapsed(start);
    assert_eq!(incremental.reparsed, 1);
    let mut timings = serde_json::Map::new();
    for (name, args) in [
        ("symbol_search", json!({"query":"AuthService"})),
        ("related_files", json!({"path":"src/session.ts"})),
        (
            "code_query",
            json!({"query":"Where is AuthService defined and what uses it?"}),
        ),
    ] {
        let start = Instant::now();
        manager.query(name, &args).unwrap();
        timings.insert(format!("{name}_ms"), json!(elapsed(start)));
    }
    let cases = [
        (
            "A",
            "AuthService",
            vec!["src/auth.ts", "src/session.ts"],
            vec![(
                "code_query",
                json!({"query":"Where is AuthService defined and what uses it?"}),
            )],
            vec!["src/auth.ts", "src/session.ts", "AuthService"],
        ),
        (
            "B",
            "validateSession",
            vec![
                "src/session.ts",
                "src/auth.ts",
                "src/api/router.ts",
                "tests/session.test.ts",
                "tsconfig.json",
            ],
            vec![("related_files", json!({"path":"src/session.ts"}))],
            vec![
                "src/auth.ts",
                "src/api/router.ts",
                "tests/session.test.ts",
                "tsconfig.json",
            ],
        ),
        (
            "C",
            "router",
            vec!["src/api/index.ts", "src/api/router.ts", "src/session.ts"],
            vec![
                ("repo_map", json!({"path":"src/api"})),
                ("file_symbols", json!({"path":"src/api/router.ts"})),
                ("file_dependencies", json!({"path":"src/api/router.ts"})),
            ],
            vec![
                "src/api/index.ts",
                "src/api/router.ts",
                "src/session.ts",
                "router",
            ],
        ),
    ];
    let mut scenarios = Vec::new();
    for (name, pattern, reads, calls, expected) in cases {
        let (old, old_content) = baseline(repo.path(), pattern, &reads);
        let (new, new_content) = indexed(&manager, &calls);
        assert!(
            expected
                .iter()
                .all(|needle| old_content.contains(needle) || reads.contains(needle)),
            "baseline {name}"
        );
        assert!(
            expected.iter().all(|needle| new_content.contains(needle)),
            "index {name}: {new_content}"
        );
        assert!(
            new_content.len() < old_content.len(),
            "context reduction {name}"
        );
        scenarios.push(json!({"scenario":name,"baseline":old,"indexed":new,"correctness":"all expected evidence present"}));
    }
    println!("{}", serde_json::to_string_pretty(&json!({"fixture_files":200,"fixture_bytes":source_bytes,
        "profile":"test/debug","startup_measurement":"controller construction plus status, 100 iterations; no scan",
        "disabled_startup_ms":disabled_startup_ms,"unused_startup_ms":unused_startup_ms,
        "cold_index_ms":cold_ms,"warm_persistent_load_ms":warm_ms,"incremental_ms":incremental_ms,
        "cold_reparsed":cold.reparsed,"warm_reparsed":warm.reparsed,"edit_reparsed":incremental.reparsed,
        "cache_bytes":fs::metadata(manager.cache_path().unwrap()).unwrap().len(),
        "warm_validation_source_bytes":warm.bytes_read,"peak_process_memory_bytes":peak_memory_bytes(),
        "query_timings":timings,"scenarios":scenarios,
        "measurement_notes":"Explicit reads measure model-requested file contents, not filesystem I/O. Index refresh hashes source files. Tokens estimated as UTF-8 bytes/4. Peak memory covers the entire test process. Synthetic fixture; not a production performance claim."})).unwrap());
}
