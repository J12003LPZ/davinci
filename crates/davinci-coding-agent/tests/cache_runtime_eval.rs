//! Explicit offline measurement, never calls a model or a network service.
use davinci_coding_agent::native_extensions::NativeExtensionHost;
use std::time::Instant;

#[test]
#[ignore = "explicit local cache benchmark"]
fn ordinary_host_and_read_baseline() {
    let root = tempfile::tempdir().unwrap();
    let agent = tempfile::tempdir().unwrap();
    let source = "export function authenticate(user) { return user.session; }\n".repeat(600);
    std::fs::write(root.path().join("auth.ts"), &source).unwrap();
    let start = Instant::now();
    for i in 0..30 {
        std::hint::black_box(NativeExtensionHost::new_with_agent_dir(
            format!("cache-eval-{i}"),
            root.path(),
            Some(agent.path()),
        ));
    }
    let host_ms = start.elapsed().as_secs_f64() * 1000.0 / 30.0;
    let context = davinci_agent::ToolContext::default();
    let start = Instant::now();
    for _ in 0..100 {
        std::hint::black_box(davinci_agent::runtime::cache::digest(source.as_bytes()));
    }
    let hash_ms = start.elapsed().as_secs_f64() * 10.0;
    let start = Instant::now();
    for _ in 0..100 {
        std::hint::black_box(
            davinci_agent::runtime::cache::read_current_file(
                root.path(),
                std::path::Path::new("auth.ts"),
                50_000,
                || Ok(()),
            )
            .unwrap(),
        );
    }
    let confined_read_ms = start.elapsed().as_secs_f64() * 10.0;
    let args = serde_json::json!({"path":"auth.ts","offset":1,"limit":2000});
    let start = Instant::now();
    for _ in 0..100 {
        let result =
            davinci_agent::tools::execute_tool_with(root.path(), "read", &args, &context).unwrap();
        assert!(result.content.contains("authenticate"));
    }
    println!(
        "{}",
        serde_json::json!({"host_initialization_mean_ms":host_ms,
        "ordinary_read_mean_ms":start.elapsed().as_secs_f64()*1000.0/100.0,"source_bytes":source.len(),"read_calls":100})
    );
    println!(
        "{}",
        serde_json::json!({"hash_ms":hash_ms,"confined_read_ms":confined_read_ms,"stats":context.cache.stats()})
    );
}
