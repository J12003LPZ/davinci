//! Simulated relaunch matrix: ten fresh hosts over one agent directory while
//! Ollama is missing, healthy, missing a model, and the config is corrupt.
//! Offline: loopback fakes only, no real `ollama`, no download. The real
//! Windows / real-Ollama matrix is not covered here (see docs/setup.md).

use davinci_coding_agent::native_extensions::service_supervisor::{
    ManagedServiceSupervisor, OllamaLauncher, OwnedProcess,
};
use davinci_coding_agent::native_extensions::{
    MemoryKind, NativeExtensionHost, VectorMemory, VectorMemoryConfig,
};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;

/// No `ollama` on this pretend machine, and nothing may be started.
struct NoOllama;
impl OllamaLauncher for NoOllama {
    fn find(&self) -> Option<std::path::PathBuf> {
        None
    }
    fn version(&self, _: &Path) -> Option<String> {
        None
    }
    fn spawn(&self, _: &Path, _: &Path) -> Result<Box<dyn OwnedProcess>, String> {
        Err("no process may be started in this test".into())
    }
}

/// Answers `/api/tags` with `models` and every embed call with 8 numbers.
fn fake(models: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            std::thread::spawn(move || {
                let mut buffer = vec![0u8; 65536];
                let mut read = 0;
                while let Ok(n) = stream.read(&mut buffer[read..]) {
                    if n == 0 {
                        break;
                    }
                    read += n;
                    let text = String::from_utf8_lossy(&buffer[..read]).to_string();
                    if let Some(split) = text.find("\r\n\r\n") {
                        let length = text[..split]
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        if read >= split + 4 + length {
                            break;
                        }
                    }
                }
                let text = String::from_utf8_lossy(&buffer[..read]).to_string();
                let reply = if text.starts_with("GET /api/tags") {
                    models.to_string()
                } else {
                    "{\"embeddings\":[[0.25,1.25,2.25,3.25,4.25,0.25,1.25,2.25]]}".to_string()
                };
                let _ = stream.write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                        reply.len()
                    )
                    .as_bytes(),
                );
            });
        }
    });
    url
}

fn write_config(agent: &Path, url: &str) {
    fs::write(
        agent.join("vector-memory.json"),
        format!(
            "{{\"ollamaUrl\":\"{url}\",\"embeddingDimensions\":8,\"requestTimeoutSeconds\":2}}"
        ),
    )
    .unwrap();
}

fn launch(agent: &Path, project: &Path) -> serde_json::Value {
    let mut host = NativeExtensionHost::new_with_agent_dir("matrix", project, Some(agent))
        .with_supervisor(std::sync::Arc::new(ManagedServiceSupervisor::new(
            Box::new(NoOllama),
        )));
    host.command("setup", "check --json").unwrap().unwrap()
}

#[test]
fn repeated_restart_reconciles_without_manual_commands() {
    let agent = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let healthy = fake("{\"models\":[{\"name\":\"embeddinggemma:latest\"}]}");
    let no_model = fake("{\"models\":[]}");
    let down = "http://127.0.0.1:9".to_string();

    // One real record written before any relaunch; its bytes must survive all of them.
    let mut seed = VectorMemory::with_config(
        project.path().to_path_buf(),
        VectorMemoryConfig {
            ollama_url: down.clone(),
            ..VectorMemoryConfig::default()
        },
    );
    seed.mark_dense_offline();
    seed.index_learning_memory(
        "the build uses cargo with the pinned toolchain",
        MemoryKind::Fact,
        0.8,
        0.9,
        "session",
        1,
        None,
    )
    .unwrap();
    let records = seed.local_path();
    let seeded = fs::read(&records).unwrap();
    assert!(!seeded.is_empty(), "the seed record was not written");
    write_config(agent.path(), &down);
    let first = launch(agent.path(), project.path());
    assert_eq!(first["health"][0]["code"], "INSTALL_REQUIRED", "{first}");

    let healthy_cfg = |a: &Path| write_config(a, &healthy);
    type Scenario<'a> = (&'a str, Box<dyn Fn(&Path) + 'a>, &'a str);
    let scenarios: Vec<Scenario> = vec![
        (
            "ollama missing",
            Box::new(|a| write_config(a, &down)),
            "INSTALL_REQUIRED",
        ),
        // Healthy server, but the seeded record has no vector yet: `check` says so
        // and does not repair by itself.
        ("healthy", Box::new(healthy_cfg), "PROJECTION_STALE"),
        (
            "model missing",
            Box::new(|a| write_config(a, &no_model)),
            "MODEL_MISSING",
        ),
        (
            "corrupt config",
            Box::new(|a| fs::write(a.join("vector-memory.json"), b"{ nope").unwrap()),
            "CONFIG_CORRUPT",
        ),
    ];
    for round in 0..10 {
        let (name, setup, code) = &scenarios[round % scenarios.len()];
        setup(agent.path());
        let config_before = fs::read(agent.path().join("vector-memory.json")).unwrap();
        let report = launch(agent.path(), project.path());
        let health = &report["health"][0];
        if code.is_empty() {
            assert_eq!(health["state"], "ready", "round {round} {name}: {report}");
        } else {
            assert_eq!(health["code"], *code, "round {round} {name}: {report}");
            assert_ne!(health["state"], "ready", "round {round} {name}");
        }
        // No relaunch rewrote the user's config, invented a consent or lost data.
        assert_eq!(
            fs::read(agent.path().join("vector-memory.json")).unwrap(),
            config_before,
            "round {round} {name}"
        );
        assert!(
            !agent.path().join("service-consent.json").exists(),
            "round {round} {name}"
        );
        assert!(
            !agent.path().join("ollama-start.lock").exists(),
            "round {round} {name}"
        );
        assert_eq!(fs::read(&records).unwrap(), seeded, "round {round} {name}");
    }
}
