//! Local release fixture, compiled against both source revisions by the setup
//! documented in resource-lifecycle.md. No provider or network requests.
use serde_json::json;
use std::{
    io::Write,
    sync::{atomic::AtomicBool, Arc},
    time::{Duration, Instant},
};

// Include the unmodified production process module from each source tree.
// Only this local fixture boundary is stubbed; permission behavior is covered
// by product tests, not inferred from these allocator/pipe measurements.
mod execution_boundary {
    pub fn record_legacy_execution(_: &str) -> Result<(), String> {
        Ok(())
    }
    pub fn require_executor(_: &str) -> Result<(), String> {
        Ok(())
    }
}
mod js_host;
#[cfg(feature = "candidate")]
mod settings {
    pub use davinci_coding_agent::settings::ExtensionRuntimeConfig;
}
mod graph {
    pub mod process;
    #[cfg(feature = "candidate")]
    pub mod process_output;
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let workload = args.get(1).map(String::as_str).unwrap_or("transcript");
    if workload == "emit" {
        let chunk = [b'x'; 16 * 1024];
        for _ in 0..2048 {
            std::io::stdout().write_all(&chunk).unwrap();
        }
        return;
    }
    let started = Instant::now();
    let result = match workload {
        "transcript" => {
            use davinci_tui::davinci::{
                model::{Entry, Model},
                theme::{ColorDepth, Theme},
                views::transcript,
            };
            let mut model = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 96, 40, true);
            let count: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(50_000);
            for i in 0..count {
                model.transcript.push(Entry::Prose(format!(
                    "Answer {i}: unchanged local benchmark evidence."
                )));
            }
            let view = transcript::conversation_rows(&model, 96);
            let rows = view.len();
            let tail = view.slice(rows.saturating_sub(30)..rows);
            let checksum: usize = tail
                .iter()
                .flat_map(|line| &line.spans)
                .map(|span| span.content.len())
                .sum();
            let cold = started.elapsed().as_nanos();
            let mut warm = Vec::new();
            for _ in 0..5 {
                let tick = Instant::now();
                let view = transcript::conversation_rows(&model, 96);
                assert_eq!(view.len(), rows);
                assert_eq!(view.slice(rows.saturating_sub(30)..rows), tail);
                warm.push(tick.elapsed().as_nanos());
            }
            println!(
                "{}",
                json!({"workload":workload,"entries":count,"rows":rows,"checksum":checksum,"coldNs":cold,"warmNs":warm})
            );
            std::thread::sleep(Duration::from_millis(150));
            std::hint::black_box(view);
            return;
        }
        "cache" => {
            use davinci_agent::runtime::cache::{
                CacheConfig, CacheKey, CacheNamespace, CachePolicy, CacheRequest, CacheRuntime,
            };
            let cache = CacheRuntime::new(
                CacheConfig {
                    max_entries: 2048,
                    persistent_enabled: false,
                    ..Default::default()
                },
                None,
            );
            for i in 0..12_000_u64 {
                let request = CacheRequest::new(
                    CacheKey::new(
                        CacheNamespace::Ast,
                        format!("entry-{i}"),
                        1,
                        "fixture",
                        vec![],
                    ),
                    CachePolicy::MemoryOnly,
                );
                assert_eq!(
                    *cache
                        .get_or_compute(&request, || Ok(()), None, || Ok(i))
                        .unwrap(),
                    i
                );
            }
            json!({"workload":workload,"coldNs":started.elapsed().as_nanos(),"operations":12000})
        }
        "output" => {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command.arg("emit");
            davinci_sys::process::set_own_process_group(&mut command);
            let mut largest = 0usize;
            let mut delivered = 0u64;
            let outcome = graph::process::run_child(
                command,
                &Arc::new(AtomicBool::new(false)),
                0,
                |line| {
                    largest = largest.max(line.len());
                    delivered += line.len() as u64;
                },
                |_| {},
            )
            .unwrap();
            assert_eq!(outcome.exit_code, 0);
            #[cfg(feature = "candidate")]
            {
                use std::io::Read;
                delivered = 0;
                for spool in outcome.output_spools {
                    let mut file = std::fs::File::open(&spool.path).unwrap();
                    let mut bytes = [0u8; 16 * 1024];
                    loop {
                        let n = file.read(&mut bytes).unwrap();
                        if n == 0 {
                            break;
                        }
                        assert!(bytes[..n].iter().all(|b| *b == b'x'));
                        delivered += n as u64;
                    }
                    std::fs::remove_file(spool.path).unwrap();
                }
            }
            assert_eq!(delivered, 32 * 1024 * 1024);
            json!({"workload":workload,"coldNs":started.elapsed().as_nanos(),"retainedOutputBytes":delivered,"maxCallbackBytes":largest})
        }
        "js_idle" | "js_active" => {
            let dir = tempfile::tempdir().unwrap();
            let module = dir.path().join("index.js");
            std::fs::write(&module, "module.exports = pi => { let calls=0; pi.on('tool_call', async event => { if(event.delay) await new Promise(resolve => setTimeout(resolve, event.delay)); return {pid:process.pid,count:++calls}; }); }; ").unwrap();
            #[cfg(feature = "candidate")]
            js_host::configure_js_runtime(settings::ExtensionRuntimeConfig {
                idle_timeout_ms: 1000,
                ..Default::default()
            })
            .unwrap();
            js_host::run_persistent_js_extension(&module, "load", &json!({})).unwrap();
            let result =
                js_host::run_persistent_js_extension(&module, "emit", &json!({"type":"tool_call"}))
                    .unwrap();
            let result = result.result.unwrap();
            let pid = result["pid"].as_u64().unwrap() as u32;
            assert_eq!(result["count"], 1);
            let before = process_rss(pid);
            if workload == "js_active" {
                let tick = Instant::now();
                let result = js_host::run_persistent_js_extension(
                    &module,
                    "emit",
                    &json!({"type":"tool_call","delay":1500}),
                )
                .unwrap()
                .result
                .unwrap();
                assert_eq!(result["pid"], pid);
                assert_eq!(result["count"], 2);
                let active_ns = tick.elapsed().as_nanos();
                js_host::shutdown_js_pool();
                println!(
                    "{}",
                    json!({"workload":workload,"activeNs":active_ns,"sameProcess":true,"configuredIdleMs":1000})
                );
                return;
            }
            std::thread::sleep(Duration::from_millis(2300));
            let after = process_rss(pid);
            js_host::shutdown_js_pool();
            json!({"workload":workload,"nodeRssBeforeIdleBytes":before,"nodeRssAfterIdleBytes":after,"idleWaitMs":2300,"configuredIdleMs":1000})
        }
        "lsp_idle" | "lsp_active" => {
            use davinci_agent::{PermissionMode, PermissionPolicy, PermissionState};
            use davinci_coding_agent::native_extensions::language_intelligence::{
                LanguageIntelligence, LanguageIntelligenceConfig,
            };
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("package.json"), "{\"private\":true}").unwrap();
            std::fs::write(dir.path().join("a.ts"), "hello").unwrap();
            let node = davinci_sys::process::resolve_program("node")
                .canonicalize()
                .unwrap();
            let mut config = LanguageIntelligenceConfig::default();
            // Accelerated fixture timeout bypasses user parser's 30-second minimum.
            config.idle_timeout_ms = 1000;
            config.rust.enabled = false;
            config.python.enabled = false;
            let events = dir.path().join("events.jsonl");
            config.typescript.server = Some(serde_json::from_value(json!({"program":node,"args":[concat!(env!("CARGO_MANIFEST_DIR"), "/src/language-server.cjs"),if workload == "lsp_active" {"delayed-query"} else {"normal"},events]})).unwrap());
            let release = if workload == "lsp_active" {
                Some(std::thread::spawn(move || {
                    let until = Instant::now() + Duration::from_secs(10);
                    while !std::fs::read_to_string(&events)
                        .unwrap_or_default()
                        .contains("textDocument/hover")
                    {
                        assert!(Instant::now() < until);
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    std::thread::sleep(Duration::from_millis(1500));
                    std::fs::write(events.with_extension("jsonl.release"), "ready").unwrap();
                }))
            } else {
                None
            };
            let manager = LanguageIntelligence::new(dir.path(), config);
            let mut policy = PermissionPolicy::new(PermissionMode::AlwaysApprove);
            policy.project_trusted = true;
            manager.set_permissions(Some(Arc::new(PermissionState::new(policy))));
            let response = manager
                .execute("lsp_hover", &json!({"path":"a.ts","line":1,"column":1}))
                .unwrap();
            assert!(!response.is_error, "{}", response.content);
            if let Some(release) = release {
                release.join().unwrap();
                assert_eq!(manager.status()["sessions"][0]["starts"], 1);
                let active_ns = started.elapsed().as_nanos();
                manager.shutdown();
                println!(
                    "{}",
                    json!({"workload":workload,"activeNs":active_ns,"sameProcess":true,"configuredIdleMs":1000})
                );
                return;
            }
            let status = manager.status();
            let pid = status["sessions"][0]["pid"].as_u64().unwrap() as u32;
            let before = process_rss(pid);
            std::thread::sleep(Duration::from_millis(2300));
            let after = process_rss(pid);
            manager.shutdown();
            json!({"workload":workload,"nodeRssBeforeIdleBytes":before,"nodeRssAfterIdleBytes":after,"idleWaitMs":2300,"configuredIdleMs":1000})
        }
        "host" => {
            use davinci_coding_agent::native_extensions::NativeExtensionHost;
            let cwd = tempfile::tempdir().unwrap();
            let agent = tempfile::tempdir().unwrap();
            std::env::set_var("DAVINCI_CODING_AGENT_DIR", agent.path());
            std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
            #[cfg(not(feature = "candidate"))]
            let host =
                NativeExtensionHost::new_with_agent_dir("fixture", cwd.path(), Some(agent.path()));
            #[cfg(feature = "candidate")]
            let host = {
                use davinci_coding_agent::native_extensions::{
                    graph::GraphWorkerContext, HostStartupRole,
                };
                let role = GraphWorkerContext::from_parts(
                    Some("classifier"),
                    Some("classification"),
                    Some("/tmp/fixture-artifact"),
                    None,
                )
                .unwrap();
                NativeExtensionHost::new_with_settings(
                    "fixture",
                    cwd.path(),
                    Some(agent.path()),
                    Default::default(),
                    HostStartupRole::GraphWorker(&role),
                )
            };
            let result = json!({"workload":workload,"coldNs":started.elapsed().as_nanos(),"learningStoreCreated":agent.path().join("learning").exists()});
            std::thread::sleep(Duration::from_millis(150));
            std::hint::black_box(host);
            result
        }
        _ => panic!("unknown workload"),
    };
    println!("{result}");
}

fn process_rss(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields: Vec<_> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    if matches!(fields.first().copied(), Some("Z" | "X")) {
        return None;
    }
    let pages: u64 = fields.get(21)?.parse().ok()?;
    #[cfg(unix)]
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64;
    #[cfg(not(unix))]
    let page_size = 4096;
    Some(pages * page_size)
}
