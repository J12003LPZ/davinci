//! WOR-51: a stdio MCP server that stops reading its stdin must not block a
//! call, a notification or teardown past the call deadline.

use std::collections::BTreeMap;
use std::process::Child;
use std::time::{Duration, Instant};

use davinci_mcp::{RpcTransport, StdioTransport};
use serde_json::json;

const CHILD_LIFETIME_SECS: u64 = 20;

/// A child that keeps its stdin pipe open but never reads from it.
fn spawn_deaf_server() -> (StdioTransport, Child) {
    let (command, args): (&str, Vec<String>) = if cfg!(windows) {
        (
            "ping",
            vec![
                "-n".into(),
                (CHILD_LIFETIME_SECS + 1).to_string(),
                "127.0.0.1".into(),
            ],
        )
    } else {
        ("sleep", vec![CHILD_LIFETIME_SECS.to_string()])
    };
    let cwd = std::env::temp_dir();
    StdioTransport::spawn(command, &args, &BTreeMap::new(), &cwd).expect("spawn deaf server")
}

/// Far larger than any OS pipe buffer, so `write_all` cannot complete.
fn oversized_payload() -> serde_json::Value {
    json!({ "blob": "x".repeat(8 * 1024 * 1024) })
}

fn kill(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn wor51_call_to_server_not_reading_stdin_times_out() {
    let (mut transport, child) = spawn_deaf_server();
    transport.set_call_timeout(Duration::from_millis(500));

    let started = Instant::now();
    let error = transport
        .call("tools/call", oversized_payload())
        .expect_err("a write the server never reads cannot succeed");
    let elapsed = started.elapsed();
    kill(child);

    assert!(
        elapsed < Duration::from_secs(5),
        "call blocked for {elapsed:?} on a full stdin pipe"
    );
    let message = error.to_string();
    assert!(message.contains("stdin"), "{message}");
    assert!(message.contains("timed out"), "{message}");
}

#[test]
fn wor51_notification_to_server_not_reading_stdin_times_out() {
    let (mut transport, child) = spawn_deaf_server();
    transport.set_call_timeout(Duration::from_millis(500));

    let started = Instant::now();
    let error = transport
        .notify("notifications/progress", oversized_payload())
        .expect_err("a write the server never reads cannot succeed");
    let elapsed = started.elapsed();
    kill(child);

    assert!(
        elapsed < Duration::from_secs(5),
        "notify blocked {elapsed:?}"
    );
    assert!(error.to_string().contains("timed out"), "{error}");
}

#[test]
fn wor51_later_calls_fail_fast_while_stdin_is_still_blocked() {
    let (mut transport, child) = spawn_deaf_server();
    transport.set_call_timeout(Duration::from_millis(300));
    assert!(transport.call("tools/call", oversized_payload()).is_err());

    // The first line is still stuck in the pipe; a small follow-up call must
    // neither hang nor interleave with it.
    let started = Instant::now();
    assert!(transport.call("ping", json!({})).is_err());
    assert!(started.elapsed() < Duration::from_secs(5));

    // Dropping the transport while the writer is stuck must not hang either.
    let started = Instant::now();
    drop(transport);
    assert!(started.elapsed() < Duration::from_secs(1));
    kill(child);
}
