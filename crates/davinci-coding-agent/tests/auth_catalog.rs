//! Exercise the installed CLI boundary with isolated, synthetic credentials.
use std::{fs, process::Command};

#[test]
fn credential_directory_eval_keeps_logged_in_models_discoverable() {
    for scenario in ["legacy", "current", "davinci_override", "pi_override"] {
        let home = tempfile::tempdir().unwrap();
        let current = home.path().join(".davinci/agent");
        let legacy = home.path().join(".pi/agent");
        fs::create_dir_all(&current).unwrap();
        fs::create_dir_all(&legacy).unwrap();
        let auth_dir = match scenario {
            "legacy" => legacy.clone(),
            "current" => current.clone(),
            _ => home.path().join("explicit"),
        };
        fs::create_dir_all(&auth_dir).unwrap();
        let payload = base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            br#"{"https://api.openai.com/auth":{"chatgpt_account_id":"fixture-account"}}"#,
        );
        fs::write(
            auth_dir.join("auth.json"),
            serde_json::json!({
                "openai-codex": {"type": "oauth", "access": format!("e30.{payload}.signature")}
            })
            .to_string(),
        )
        .unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_davinci"));
        command
            .current_dir(home.path())
            .env("USERPROFILE", home.path())
            .env("HOME", home.path())
            .env_remove("DAVINCI_CODING_AGENT_DIR")
            .env_remove("PI_CODING_AGENT_DIR")
            .args([
                "--offline",
                "--no-extensions",
                "--list-models",
                "openai-codex",
            ]);
        match scenario {
            "davinci_override" => {
                command
                    .env("DAVINCI_CODING_AGENT_DIR", &auth_dir)
                    .env("PI_CODING_AGENT_DIR", &legacy);
            }
            "pi_override" => {
                command.env("PI_CODING_AGENT_DIR", &auth_dir);
            }
            _ => {}
        }
        let output = command.output().unwrap();
        assert!(output.status.success(), "{scenario}: CLI failed");
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(
            stdout.contains("openai-codex") && stdout.contains("gpt-5.6-luna"),
            "{scenario}: model missing: {stdout}"
        );
    }
}
