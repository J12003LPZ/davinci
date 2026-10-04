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
        fs::write(
            auth_dir.join("auth.json"),
            serde_json::json!({
                "openai-codex": {
                    "type": "oauth",
                    "access": "fixture-plan-access",
                    "expires": u64::MAX,
                    "env": {
                        "OPENAI_SIWC_CLIENT_ID": "oaiapp_fixture",
                        "OPENAI_SIWC_EXT_AGENT_HOST_ID": "urn:uuid:00000000-0000-4000-8000-000000000001",
                        "OPENAI_SIWC_ISSUER": "https://auth.openai.com",
                        "OPENAI_SIWC_SUBJECT": "fixture-subject",
                        "OPENAI_SIWC_EMAIL": "fixture@example.test",
                        "OPENAI_SIWC_ID_TOKEN": "fixture-id-token",
                        "OPENAI_SIWC_SCOPES": "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct",
                        "OPENAI_SIWC_RESOURCE": "https://api.openai.com/v1"
                    }
                }
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
