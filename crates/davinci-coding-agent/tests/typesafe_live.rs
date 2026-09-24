//! Live TypeSafe acceptance: real network calls with the user's stored
//! credential, through the same code the runtime uses. Ignored by default so
//! the offline gate never spends API calls.
//!
//! Run: cargo test -p davinci-coding-agent --test typesafe_live -- --ignored --nocapture --test-threads=1
//!
//! The credential is resolved exactly as DaVinci does (TYPESAFE_API_KEY, else
//! the `typesafe` entry in auth.json) and is never printed.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use davinci_agent::decision::provider::DecisionProvider;
use davinci_agent::decision::response::DecisionAnswer;
use davinci_agent::decision::HARD_DECISION_BUDGET;
use davinci_ai::AuthStorage;
use davinci_coding_agent::decision_providers::typesafe::{resolve_api_key, TypeSafeProvider};
use davinci_coding_agent::decision_state::{build_request_with_metadata, DecisionMetadata};

fn auth_path() -> PathBuf {
    for var in ["DAVINCI_CODING_AGENT_DIR", "PI_CODING_AGENT_DIR"] {
        if let Some(dir) = std::env::var_os(var) {
            return PathBuf::from(dir).join("auth.json");
        }
    }
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .expect("home directory");
    let davinci = home.join(".davinci/agent/auth.json");
    if davinci.exists() {
        davinci
    } else {
        home.join(".pi/agent/auth.json")
    }
}

fn live_key() -> String {
    let auth = AuthStorage::open(&auth_path()).expect("auth store opens");
    resolve_api_key(&auth)
        .expect("stored TypeSafe credential is well formed")
        .expect("a TypeSafe credential is configured; set it in /settings")
}

fn all_capabilities() -> Vec<String> {
    [
        "browser_open",
        "git_symbol_history",
        "package_info",
        "test_impacted",
        "impact_analyze",
        "verification_plan",
    ]
    .map(str::to_owned)
    .to_vec()
}

#[test]
#[ignore = "live TypeSafe call; needs a configured credential"]
fn live_credential_probe_passes_the_runtime_parser() {
    let started = Instant::now();
    TypeSafeProvider::validate_api_key(&live_key()).expect("live probe accepted");
    println!("LIVE_PROBE ok in {} ms", started.elapsed().as_millis());
}

#[test]
#[ignore = "live TypeSafe call; needs a configured credential"]
fn live_routing_requests_parse_and_report_answers() {
    let provider = TypeSafeProvider::new(live_key());
    let tasks = [
        (
            "ui",
            "The login button on the React sign-in page is misaligned on mobile; fix the CSS.",
            &["login.tsx", "login.css"][..],
        ),
        ("typo", "Fix the typo 'recieve' in the README.", &["README.md"][..]),
        (
            "regression",
            "Checkout totals became wrong sometime last week; find which commit broke the tax calculation.",
            &["checkout.rs"][..],
        ),
        (
            "dependency",
            "Upgrade serde to the latest major version and fix any API breakage.",
            &["Cargo.toml", "lib.rs"][..],
        ),
        (
            "contract",
            "Rename the `user_id` field to `account_id` in the shared API schema used by every service.",
            &["schema.ts", "handlers.go"][..],
        ),
    ];
    let mut failures = Vec::new();
    for (name, task, paths) in tasks {
        let metadata = DecisionMetadata::from_workspace(
            std::path::Path::new("."),
            &paths.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
            &all_capabilities(),
        );
        let request = build_request_with_metadata(
            format!("live-{name}"),
            task,
            davinci_agent::decision::risk::DecisionRisk::Planning,
            metadata,
        );
        let bytes = request.validate_size().expect("request shape").len();
        let started = Instant::now();
        let result = provider.evaluate(&request, HARD_DECISION_BUDGET);
        let latency = started.elapsed().as_millis();
        match result {
            Ok(response) => {
                let answers = response
                    .answers
                    .iter()
                    .map(|(id, answer)| match answer {
                        DecisionAnswer::Noul { value } => format!("{id}={value:.2}"),
                        DecisionAnswer::Choice {
                            choice, confidence, ..
                        } => format!("{id}={choice}(c={confidence:.2})"),
                        DecisionAnswer::Score {
                            score,
                            levels,
                            confidence,
                            ..
                        } => format!("{id}={score:.2}/{}(c={confidence:.2})", levels - 1),
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                println!(
                    "LIVE {name} ok {latency}ms req={bytes}B model={} usage={:?}/{:?} :: {answers}",
                    response.model.as_deref().unwrap_or("?"),
                    response.input_tokens,
                    response.output_tokens,
                );
            }
            Err(error) => {
                println!("LIVE {name} FAILED {latency}ms req={bytes}B :: {error:?}");
                failures.push(name);
            }
        }
        // Sequential calls reuse one pooled connection; pause only to stay
        // polite to the rate limiter.
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(failures.is_empty(), "live requests failed: {failures:?}");
}
