//! `--output-schema <file>` driven through the packaged binary against a
//! loopback chat-completions provider (a `models.json` custom provider on
//! 127.0.0.1). The server records every request body, so the tests can count
//! the repair turn exactly and check the `response_format` on the wire.
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output, Stdio},
    sync::{Arc, Mutex},
};

const SCHEMA: &str = r#"{
  "type": "object",
  "properties": {"ok": {"type": "boolean"}, "items": {"type": "array", "items": {"type": "string"}}},
  "required": ["ok"],
  "additionalProperties": false
}"#;

/// Answers each POST with the next scripted reply (the last one repeats) and
/// keeps the request bodies it saw.
fn provider(replies: &[&str]) -> (String, Arc<Mutex<Vec<Value>>>) {
    provider_with_api(replies, false)
}

fn provider_with_api(replies: &[&str], anthropic: bool) -> (String, Arc<Mutex<Vec<Value>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let replies: Vec<String> = replies.iter().map(|reply| reply.to_string()).collect();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorded = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut length = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if let Some((name, value)) = line.split_once(':') {
                    if name.eq_ignore_ascii_case("content-length") {
                        length = value.trim().parse().unwrap_or(0);
                    }
                }
            }
            let mut body = vec![0u8; length];
            reader.read_exact(&mut body).unwrap();
            let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            let streaming = body["stream"] == json!(true);
            let index = {
                let mut seen = recorded.lock().unwrap();
                seen.push(body);
                seen.len() - 1
            };
            let text = &replies[index.min(replies.len() - 1)];
            let response = if let Some(error) = text.strip_prefix("HTTP 400 ") {
                format!(
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{error}",
                    error.len()
                )
            } else if anthropic {
                let events = [
                    json!({"type":"message_start","message":{"id":"fixture","type":"message","role":"assistant","model":"demo","content":[],"usage":{"input_tokens":1,"output_tokens":0}}}),
                    json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
                    json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":text}}),
                    json!({"type":"content_block_stop","index":0}),
                    json!({"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":1}}),
                    json!({"type":"message_stop"}),
                ];
                let body = events
                    .iter()
                    .map(|event| {
                        format!(
                            "event: {}\ndata: {event}\n\n",
                            event["type"].as_str().unwrap()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("");
                format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{body}")
            } else if streaming {
                let frame = json!({"choices": [{"index": 0, "delta": {"content": text}, "finish_reason": "stop"}]});
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {frame}\n\ndata: [DONE]\n\n"
                )
            } else {
                let body = json!({"choices": [{"index": 0, "message": {"role": "assistant", "content": text}, "finish_reason": "stop"}]})
                    .to_string();
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
            };
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    (base, seen)
}

fn davinci(root: &Path, base_url: &str) -> Command {
    davinci_with_api(root, base_url, "openai-completions")
}

fn davinci_with_api(root: &Path, base_url: &str, api: &str) -> Command {
    davinci_options(root, base_url, api, false)
}

fn davinci_options(root: &Path, base_url: &str, api: &str, read_tool: bool) -> Command {
    let config = root.join("config");
    fs::create_dir_all(&config).unwrap();
    fs::write(
        config.join("settings.json"),
        json!({"processManager": {"enabled": false}}).to_string(),
    )
    .unwrap();
    fs::write(
        config.join("models.json"),
        json!({"providers": {"local": {
            "baseUrl": base_url,
            "api": api,
            "apiKey": "sk-test",
            "models": [{"id": "demo", "name": "Demo"}]
        }}})
        .to_string(),
    )
    .unwrap();
    fs::write(root.join("schema.json"), SCHEMA).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_davinci"));
    command.env_clear();
    for key in ["PATH", "SystemRoot", "WINDIR", "TEMP", "TMP"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .current_dir(root)
        .args([
            "--no-extensions",
            "--no-skills",
            "--no-prompt-templates",
            "--no-mcp",
            "--no-session",
            "--provider",
            "local",
            "--model",
            "demo",
        ])
        .env("HOME", &config)
        .env("USERPROFILE", &config)
        .env("PI_CODING_AGENT_DIR", &config)
        .env("DAVINCI_CODING_AGENT_DIR", &config)
        .env("PI_DISABLE_NETWORK", "1")
        .env("PI_HOOKS_DRY_RUN", "1")
        .env("PI_LEARNING_DISABLE_BACKGROUND", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if read_tool {
        command.args(["--tools", "read"]);
    } else {
        command.arg("--no-tools");
    }
    command
}

fn describe(output: &Output) -> String {
    format!(
        "status {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn last_user_text(body: &Value) -> String {
    body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|message| message["role"] == "user")
        .map(|message| message["content"].as_str().unwrap_or_default().to_string())
        .unwrap_or_default()
}

#[test]
fn one_repair_turn_fixes_a_nonconforming_answer() {
    let root = tempfile::tempdir().unwrap();
    let (base, seen) = provider(&[r#"Sure! Here: {"ok": "yes"}"#, r#"{"ok": true}"#]);
    let output = davinci(root.path(), &base)
        .args([
            "--output-schema",
            "schema.json",
            "-o",
            "last.json",
            "-p",
            "report",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        r#"{"ok": true}"#
    );
    assert_eq!(
        fs::read_to_string(root.path().join("last.json")).unwrap(),
        r#"{"ok": true}"#
    );

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2, "exactly one extra request: {seen:#?}");
    let schema: Value = serde_json::from_str(SCHEMA).unwrap();
    for body in seen.iter() {
        assert_eq!(body["response_format"]["type"], "json_schema");
        assert_eq!(body["response_format"]["json_schema"]["strict"], true);
        assert_eq!(body["response_format"]["json_schema"]["schema"], schema);
    }
    let repair = last_user_text(&seen[1]);
    assert!(
        repair.contains("does not match the required JSON schema"),
        "{repair}"
    );
    assert!(repair.contains("not valid JSON"), "{repair}");
}

#[test]
fn a_second_bad_answer_fails_the_run_with_the_errors() {
    let root = tempfile::tempdir().unwrap();
    let (base, seen) = provider(&[r#"{"ok": 1}"#, r#"{"ok": true, "extra": 2}"#]);
    let output = davinci(root.path(), &base)
        .args(["--output-schema", "schema.json", "-p", "report"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("does not match --output-schema after 1 repair turn:"),
        "{}",
        describe(&output)
    );
    assert!(
        stderr.contains("- $: property \"extra\" is not allowed"),
        "{}",
        describe(&output)
    );
    // The non-conforming reply is not passed off as the answer.
    assert!(!String::from_utf8_lossy(&output.stdout).contains("extra"));

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2, "never a second repair: {seen:#?}");
    let repair = last_user_text(&seen[1]);
    assert!(
        repair.contains("- $.ok: expected boolean, got integer"),
        "{repair}"
    );
}

#[test]
fn json_mode_reports_the_check_and_accepts_a_fenced_answer() {
    let root = tempfile::tempdir().unwrap();
    let (base, seen) = provider(&["```json\n{\"ok\": false, \"items\": [\"a\"]}\n```"]);
    let output = davinci(root.path(), &base)
        .args([
            "--mode",
            "json",
            "--output-schema",
            "schema.json",
            "-o",
            "last.json",
            "-p",
            "report",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(
        fs::read_to_string(root.path().join("last.json")).unwrap(),
        r#"{"ok": false, "items": ["a"]}"#
    );
    let events: Vec<Value> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|_| panic!("not json: {line}")))
        .collect();
    let check = events
        .iter()
        .find(|event| event["type"] == "output_schema")
        .unwrap_or_else(|| panic!("{}", describe(&output)));
    assert_eq!(
        *check,
        json!({"type": "output_schema", "checked": true, "valid": true, "repairTurns": 0, "errors": []})
    );

    // A repaired run says so, so benchmark comparisons can count the turn.
    let (base, seen) = provider(&["nope", r#"{"ok": true}"#]);
    let output = davinci(root.path(), &base)
        .args([
            "--mode",
            "json",
            "--output-schema",
            "schema.json",
            "-p",
            "report",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    assert_eq!(seen.lock().unwrap().len(), 2);
    let check: Value = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|event| event["type"] == "output_schema")
        .unwrap_or_else(|| panic!("{}", describe(&output)));
    assert_eq!(check["repairTurns"], 1);
    assert_eq!(check["valid"], true);
}

#[test]
fn a_bad_schema_file_fails_before_any_request() {
    let root = tempfile::tempdir().unwrap();
    let (base, seen) = provider(&[r#"{"ok": true}"#]);

    let output = davinci(root.path(), &base)
        .args(["--output-schema", "missing.json", "-p", "report"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Error: --output-schema: could not read")
    );

    fs::write(root.path().join("broken.json"), "{\"type\": ").unwrap();
    let output = davinci(root.path(), &base)
        .args(["--output-schema", "broken.json", "-p", "report"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    assert!(String::from_utf8_lossy(&output.stderr).contains("is not valid JSON"));

    let output = davinci(root.path(), &base)
        .args(["--mode", "rpc", "--output-schema", "schema.json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("--output-schema is not supported with --mode rpc"));

    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn a_provider_that_rejects_the_schema_fails_clearly_without_a_repair() {
    let root = tempfile::tempdir().unwrap();
    let (base, seen) = provider(&[
        r#"HTTP 400 {"error":{"message":"Invalid schema for response_format 'davinci_output_schema'","type":"invalid_request_error"}}"#,
    ]);
    let output = davinci(root.path(), &base)
        .args(["--output-schema", "schema.json", "-p", "report"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Invalid schema for response_format"),
        "{}",
        describe(&output)
    );
    assert!(!stderr.contains("repair turn"), "{}", describe(&output));
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn without_the_flag_nothing_changes_on_the_wire_or_in_the_output() {
    let root = tempfile::tempdir().unwrap();
    let (base, seen) = provider(&["plain prose answer"]);
    let output = davinci(root.path(), &base)
        .args(["-p", "report"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "plain prose answer"
    );
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].get("response_format").is_none());
}

#[test]
fn anthropic_receives_schema_before_first_answer_and_repair() {
    let root = tempfile::tempdir().unwrap();
    let (base, seen) = provider_with_api(&["not json", r#"{"ok":true}"#], true);
    let output = davinci_with_api(root.path(), &base, "anthropic-messages")
        .args(["--output-schema", "schema.json", "-p", "report"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2, "{seen:#?}");
    for request in seen.iter() {
        let system = request["system"].to_string();
        assert!(system.contains("Final answer contract"), "{request}");
        assert!(
            system.contains("required") && system.contains("additionalProperties"),
            "{system}"
        );
        assert!(request.get("response_format").is_none());
    }
}

#[test]
fn unsupported_nested_assertion_fails_before_any_provider_request() {
    let root = tempfile::tempdir().unwrap();
    let (base, seen) = provider(&[r#"{"ok":true}"#]);
    let mut command = davinci(root.path(), &base);
    fs::write(
        root.path().join("schema.json"),
        r#"{"properties":{"ok":{"pattern":"x"}}}"#,
    )
    .unwrap();
    let output = command
        .args(["--output-schema", "schema.json", "-p", "report"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported schema keyword"));
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn named_file_bytes_reach_the_first_cli_provider_request() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("calc.py"), "CAPTURED_FILE_BODY = 7\n").unwrap();
    let (base, seen) = provider(&["Reviewed."]);
    let output = davinci_options(root.path(), &base, "openai-completions", true)
        .env("DAVINCI_TURN_CONTEXT", "appended")
        .env("DAVINCI_NAMED_FILES", "1")
        .args(["-p", "Review calc.py"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert!(
        seen[0]["messages"]
            .to_string()
            .contains("CAPTURED_FILE_BODY"),
        "{}",
        seen[0]
    );
}

#[test]
fn repair_diagnostics_are_not_genuine_user_file_requests() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("invoice.py"), "PRIVATE_INVOICE_BODY").unwrap();
    let (base, seen) = provider(&["{}", r#"{"invoice.py":true}"#]);
    let mut command = davinci_options(root.path(), &base, "openai-completions", true);
    fs::write(
        root.path().join("schema.json"),
        r#"{"type":"object","required":["invoice.py"]}"#,
    )
    .unwrap();
    let output = command
        .env("DAVINCI_TURN_CONTEXT", "appended")
        .env("DAVINCI_NAMED_FILES", "1")
        .args([
            "--output-schema",
            "schema.json",
            "--mode",
            "json",
            "-p",
            "Generate JSON",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    let requests = seen.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(!requests
        .iter()
        .any(|request| request.to_string().contains("PRIVATE_INVOICE_BODY")));
    let messages: Vec<Value> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|event| event["type"] == "message_start" && event["message"]["role"] == "user")
        .map(|event| event["message"].clone())
        .collect();
    let repair = messages
        .iter()
        .find(|message| {
            message["content"]
                .to_string()
                .contains("does not match the required JSON schema")
        })
        .expect("repair appears as a harness message");
    assert_ne!(repair["davinciRealUserOrigin"], json!(true));
}
