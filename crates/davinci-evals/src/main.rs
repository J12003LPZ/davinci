use clap::{Args, Parser, Subcommand};
use davinci_agent::{prompt::PromptModelPolicy, Agent, PromptProfile};
use davinci_evals::behavior::{
    aggregate_scenario_results, bootstrap_pass_delta_ci95, compare_repeated_runs, execute_scenario,
    pair_by_scenario_repetition, ArtifactRoot, BehaviorCategory, BehaviorRequirement,
    DavinciProcessConfig, DispositionedSuiteSummary, EvalRunManifest, RunDisposition,
    RunDispositionSummary, ScenarioRunResult, ScenarioRunSample, DEFAULT_PROMOTION_REPEATS,
};
use davinci_evals::behavior::{
    format_repeated_comparison_markdown, BehaviorScenario, RepeatedPairedComparison,
};
use davinci_evals::competitor::{
    comparison_class_for_metadata, evaluate_competitor_claim, format_competitor_report_markdown,
    format_competitor_suite_summary, load_claude_public_challenges, ClaudeChallengeCategory,
    ClaudeCodeHarness, ClaudePublicChallenge, CommandHarness, CompetitorClaimInputs,
    CompetitorComparisonReport, CompetitorSuiteSummary, ExternalRun, FairComparisonMetadata,
    HarnessCapabilities, MatchedRunConfig, MatchedRunResult,
};
use davinci_evals::promotion::{
    validate_promotion_evidence_against_hashes, PromptPromotionEvidence,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(
    name = "davinci-evals",
    version,
    about = "Run DaVinci behavior evaluations"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: TopLevelCommand,
}

#[derive(Debug, Subcommand)]
pub enum TopLevelCommand {
    Corpus {
        #[command(subcommand)]
        command: CorpusCommand,
    },
    Behavior {
        #[command(subcommand)]
        command: BehaviorCommand,
    },
    Competitor {
        #[command(subcommand)]
        command: CompetitorCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum CorpusCommand {
    Verify,
}

#[derive(Debug, Subcommand)]
pub enum BehaviorCommand {
    Run(BehaviorRunArgs),
    Ab(BehaviorAbArgs),
    Gate(BehaviorGateArgs),
    PromoteCheck(BehaviorPromoteCheckArgs),
}

#[derive(Debug, Args)]
pub struct BehaviorRunArgs {
    #[arg(long, default_value = "core-200")]
    pub suite: String,
    /// Run one debugging-hard scenario for bounded live diagnostics.
    #[arg(long, requires = "suite")]
    pub scenario_id: Option<String>,
    #[arg(long, default_value = "stable")]
    pub profile: String,
    #[arg(long)]
    pub artifacts: Option<PathBuf>,
    #[arg(long)]
    pub provider: Option<String>,
    #[arg(long)]
    pub model: Option<String>,
    #[arg(long)]
    pub davinci_bin: Option<PathBuf>,
    #[arg(long, default_value = "ask")]
    pub permission_mode: String,
}

#[derive(Debug, Args)]
pub struct BehaviorAbArgs {
    #[arg(long, default_value = "core-200")]
    pub suite: String,
    #[arg(long, default_value = "stable")]
    pub baseline_profile: String,
    #[arg(long, default_value = "preview")]
    pub candidate_profile: String,
    #[arg(long)]
    pub baseline_model_policy: Option<String>,
    #[arg(long)]
    pub candidate_model_policy: Option<String>,
    #[arg(long)]
    pub provider: String,
    #[arg(long)]
    pub model: String,
    #[arg(long, default_value_t = DEFAULT_PROMOTION_REPEATS)]
    pub repeats: u32,
    #[arg(long)]
    pub davinci_bin: PathBuf,
    #[arg(long, default_value = "target/behavior-evals")]
    pub artifacts: PathBuf,
    #[arg(long, default_value = "ask")]
    pub permission_mode: String,
}

#[derive(Debug, Args)]
pub struct BehaviorGateArgs {
    #[arg(long)]
    pub artifacts: PathBuf,
    #[arg(long, default_value = "ask")]
    pub permission_mode: String,
}

#[derive(Debug, Args)]
pub struct BehaviorPromoteCheckArgs {
    #[arg(long)]
    pub evidence: PathBuf,
    #[arg(long, default_value = "target/behavior-evals")]
    pub artifacts_root: PathBuf,
    #[arg(long)]
    pub candidate_prompt_hash: Option<String>,
    #[arg(long)]
    pub stable_prompt_hash: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum CompetitorCommand {
    Run(CompetitorRunArgs),
    Compare(CompetitorCompareArgs),
}

#[derive(Debug, Args)]
pub struct CompetitorRunArgs {
    #[arg(long, default_value = "claude-public-challenges")]
    pub suite: String,
    #[arg(long)]
    pub binary: PathBuf,
    #[arg(long, default_value_t = DEFAULT_PROMOTION_REPEATS)]
    pub repeats: u32,
    #[arg(long, default_value = "target/competitor-evals")]
    pub artifacts: PathBuf,
    #[arg(long)]
    pub davinci_bin: Option<PathBuf>,
    #[arg(long)]
    pub provider: Option<String>,
    #[arg(long)]
    pub model: Option<String>,
    #[arg(long, default_value_t = 120)]
    pub timeout_seconds: u64,
}

#[derive(Debug, Args)]
pub struct CompetitorCompareArgs {
    #[arg(long, default_value = "claude-public-challenges")]
    pub suite: String,
    #[arg(long, default_value_t = 3)]
    pub repeats: u32,
    #[arg(long)]
    pub artifacts: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BehaviorExecutionReport {
    run_id: String,
    profile: Option<String>,
    baseline: Option<DispositionedSuiteSummary>,
    candidate: Option<DispositionedSuiteSummary>,
    repeated: Option<RepeatedPairedComparison>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DispositionSummaryArtifact {
    baseline: RunDispositionSummary,
    candidate: Option<RunDispositionSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CompetitorSampleArtifact {
    scenario_id: String,
    repetition: u32,
    matched: Option<MatchedRunResult>,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CompetitorRawArtifact {
    suite: String,
    suite_hash: String,
    repeats: u32,
    provider: String,
    model: String,
    competitor_binary: String,
    competitor_model: String,
    models_controlled: bool,
    permission_mode: String,
    timeout_seconds: u64,
    capabilities: HarnessCapabilities,
    samples: Vec<CompetitorSampleArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CompetitorComparisonArtifact {
    metadata: FairComparisonMetadata,
    summary: CompetitorSuiteSummary,
    runtime_boundary_failures: u32,
    claim_failures: Vec<String>,
    samples: Vec<CompetitorSampleArtifact>,
}

fn load_behavior_suite(name: &str) -> Result<Vec<BehaviorScenario>, String> {
    match name {
        "core-200" => davinci_evals::behavior::load_core_200_corpus(),
        "debugging-hard" => davinci_evals::behavior::load_debugging_hard_corpus(),
        "gpt6-astra" => Ok(
            davinci_evals::behavior::load_regression_suite("gpt6-astra")?
                .into_iter()
                .map(|case| case.scenario)
                .collect(),
        ),
        _ => Err(format!(
            "unknown behavior suite '{name}'; valid suites: core-200, debugging-hard, gpt6-astra"
        )),
    }
}

fn parse_profile(name: &str) -> Result<PromptProfile, String> {
    PromptProfile::parse(name).ok_or_else(|| {
        format!("invalid prompt profile '{name}'; valid profiles: stable, preview, legacy-v1")
    })
}

fn parse_model_policy(name: Option<&str>) -> Result<Option<PromptModelPolicy>, String> {
    match name.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(None),
        Some("default") => Ok(Some(PromptModelPolicy::Default)),
        Some("gpt6-astra") => Ok(Some(PromptModelPolicy::Gpt6Astra)),
        Some(other) => Err(format!(
            "invalid model policy '{other}'; valid policies: default, gpt6-astra"
        )),
    }
}

fn stable_hash_for_ab_variant(
    profile: PromptProfile,
    provider: &str,
    model: &str,
    policy_override: Option<PromptModelPolicy>,
) -> String {
    let policy = policy_override.unwrap_or_else(|| {
        davinci_agent::prompt::model_policy::inferred_prompt_model_policy(provider, model)
    });
    let mut modules =
        davinci_agent::prompt::apply_model_policy(policy, profile, profile.bundle().modules);
    let family = davinci_agent::prompt::prompt_model_family(provider, model);
    if let Some(adapter) = davinci_agent::prompt::provider_adapter(family) {
        modules.push(adapter);
    }
    davinci_agent::prompt::compose_modules(&modules)
        .manifest
        .stable_sha256
}

fn ab_variant_label(profile: PromptProfile, policy: Option<PromptModelPolicy>) -> String {
    match policy {
        Some(policy) => format!("{} · {} v{}", profile.id(), policy.id(), policy.version()),
        None => profile.id().to_string(),
    }
}

fn first_nonempty_value(requested: Option<&str>, environment_names: &[&str]) -> Option<String> {
    requested
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            environment_names.iter().find_map(|name| {
                std::env::var(name)
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
            })
        })
}

fn resolve_runtime_config(
    provider: Option<&str>,
    model: Option<&str>,
    binary: Option<&Path>,
    profile: PromptProfile,
    model_policy_override: Option<PromptModelPolicy>,
    permission_mode: &str,
    artifact_root: &Path,
) -> Result<DavinciProcessConfig, String> {
    let provider = first_nonempty_value(provider, &["DAVINCI_PROVIDER", "PI_PROVIDER"])
        .ok_or_else(|| {
            "provider is required (--provider or DAVINCI_PROVIDER/PI_PROVIDER)".to_string()
        })?;
    let model = first_nonempty_value(model, &["DAVINCI_MODEL", "PI_MODEL"])
        .ok_or_else(|| "model is required (--model or DAVINCI_MODEL/PI_MODEL)".to_string())?;
    let binary = binary
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("DAVINCI_BIN").map(PathBuf::from))
        .unwrap_or_else(|| {
            if cfg!(windows) {
                PathBuf::from("target/release/davinci.exe")
            } else {
                PathBuf::from("target/release/davinci")
            }
        });
    if permission_mode.trim().is_empty() {
        return Err("permission mode must not be empty".into());
    }

    let mut allowed_env = BTreeMap::new();
    for name in [
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "GEMINI_API_KEY",
        "GOOGLE_API_KEY",
        "ANTHROPIC_BASE_URL",
        "OPENAI_BASE_URL",
        "PI_OFFLINE",
        "PATH",
        "PATHEXT",
        "SystemRoot",
        "WINDIR",
        "ComSpec",
        "TEMP",
        "TMP",
    ] {
        if let Ok(value) = std::env::var(name) {
            allowed_env.insert(name.to_string(), value);
        }
    }

    Ok(DavinciProcessConfig {
        binary,
        launcher_args: Vec::new(),
        provider,
        model,
        prompt_profile: profile,
        prompt_model_policy_override: model_policy_override,
        permission_mode: permission_mode.to_string(),
        timeout: Duration::from_secs(300),
        clean_agent_dir: artifact_root.join("agent"),
        auth_source: {
            let path = davinci_ai::default_auth_path();
            path.is_file().then_some(path)
        },
        allowed_env,
    })
}

fn execute_suite(
    scenarios: &[BehaviorScenario],
    config: &DavinciProcessConfig,
    artifact_root: &Path,
    label: &str,
    repeats: u32,
) -> Result<Vec<(u32, ScenarioRunResult)>, String> {
    if repeats == 0 {
        return Err("repeats must be greater than zero".into());
    }
    let mut runs = Vec::with_capacity(scenarios.len() * repeats as usize);
    for repetition in 0..repeats {
        let repetition_root = artifact_root
            .join("scenarios")
            .join(label)
            .join(format!("repeat-{repetition}"));
        let artifacts = ArtifactRoot::new(&repetition_root);
        let mut repetition_results = Vec::with_capacity(scenarios.len());
        for scenario in scenarios {
            let result = execute_scenario(scenario, config, &artifacts);
            runs.push((repetition, result.clone()));
            repetition_results.push(result);
        }
        artifacts.write_json("results.json", &repetition_results)?;
    }
    Ok(runs)
}

fn aggregate_runs(runs: &[(u32, ScenarioRunResult)]) -> DispositionedSuiteSummary {
    let results: Vec<ScenarioRunResult> = runs.iter().map(|(_, result)| result.clone()).collect();
    aggregate_scenario_results(&results)
}

fn paired_samples(
    baseline: &[(u32, ScenarioRunResult)],
    candidate: &[(u32, ScenarioRunResult)],
) -> (Vec<ScenarioRunSample>, Vec<ScenarioRunSample>) {
    let candidate_by_key: BTreeMap<(String, u32), &ScenarioRunResult> = candidate
        .iter()
        .filter(|(_, result)| result.disposition == RunDisposition::BehavioralResult)
        .map(|(repetition, result)| ((result.scenario.id.clone(), *repetition), result))
        .collect();
    let mut baseline_samples = Vec::new();
    let mut candidate_samples = Vec::new();
    for (repetition, result) in baseline
        .iter()
        .filter(|(_, result)| result.disposition == RunDisposition::BehavioralResult)
    {
        let key = (result.scenario.id.clone(), *repetition);
        let Some(candidate_result) = candidate_by_key.get(&key) else {
            continue;
        };
        baseline_samples.push(ScenarioRunSample {
            scenario_id: result.scenario.id.clone(),
            repetition: *repetition,
            passed: result.score.passed,
            wall_ms: result.trace.stats.wall_ms,
            tool_calls: result.trace.stats.tool_calls,
        });
        candidate_samples.push(ScenarioRunSample {
            scenario_id: candidate_result.scenario.id.clone(),
            repetition: *repetition,
            passed: candidate_result.score.passed,
            wall_ms: candidate_result.trace.stats.wall_ms,
            tool_calls: candidate_result.trace.stats.tool_calls,
        });
    }
    (baseline_samples, candidate_samples)
}

fn hash_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().fold(
        String::with_capacity(digest.len() * 2),
        |mut output, byte| {
            write!(output, "{byte:02x}").expect("writing to a String cannot fail");
            output
        },
    )
}

fn hash_json<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_vec(value)
        .map(|bytes| hash_bytes(&bytes))
        .map_err(|error| format!("failed to encode hash input: {error}"))
}

fn git_revision() -> String {
    Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unavailable".into())
}

fn tool_surface_hash() -> Result<String, String> {
    let agent = Agent::new_builtin(PromptProfile::Stable);
    hash_json(&agent.builtin_and_mcp_specs())
}

fn run_id() -> String {
    format!("behavior-{}", Uuid::new_v4().simple())
}

fn write_run_artifacts(
    artifacts: &ArtifactRoot,
    report: &BehaviorExecutionReport,
    disposition_summary: &DispositionSummaryArtifact,
    summary_markdown: &str,
) -> Result<(), String> {
    artifacts.write_json("summary.json", report)?;
    artifacts.write_text("summary.md", summary_markdown)?;
    artifacts.write_json("disposition-summary.json", disposition_summary)?;
    artifacts.finalize_hashes()?;
    Ok(())
}

fn suite_markdown(
    run_id: &str,
    suite: &str,
    profile: &str,
    summary: &DispositionedSuiteSummary,
) -> String {
    format!(
        "# DaVinci behavior evaluation\n\n- Run: `{run_id}`\n- Suite: `{suite}`\n- Profile: `{profile}`\n\n## Behavioral results\n\n- Behavioral runs: {}\n- Passed scenarios: {}\n- Macro pass rate: {:.2}%\n\n## Run disposition\n\n- Total runs: {}\n- Infrastructure failures: {} ({:.2}%)\n- Configuration failures: {} ({:.2}%)\n",
        summary.dispositions.behavioral_runs,
        summary.behavioral.passed_scenarios,
        summary.behavioral.macro_pass_rate * 100.0,
        summary.dispositions.total_runs,
        summary.dispositions.infrastructure_failures,
        summary.dispositions.infrastructure_failure_rate() * 100.0,
        summary.dispositions.configuration_failures,
        summary.dispositions.configuration_failure_rate() * 100.0,
    )
}

fn ab_markdown(
    run_id: &str,
    suite: &str,
    baseline_profile: &str,
    candidate_profile: &str,
    baseline: &DispositionedSuiteSummary,
    candidate: &DispositionedSuiteSummary,
    repeated: Option<&RepeatedPairedComparison>,
) -> String {
    let mut output = format!(
        "# DaVinci behavior A/B evaluation\n\n- Run: `{run_id}`\n- Suite: `{suite}`\n\n| Profile | Behavioral runs | Passed | Pass rate | Infrastructure failures | Configuration failures |\n| :--- | ---: | ---: | ---: | ---: | ---: |\n| {baseline_profile} | {} | {} | {:.2}% | {} | {} |\n| {candidate_profile} | {} | {} | {:.2}% | {} | {} |\n",
        baseline.dispositions.behavioral_runs,
        baseline.behavioral.passed_scenarios,
        baseline.behavioral.macro_pass_rate * 100.0,
        baseline.dispositions.infrastructure_failures,
        baseline.dispositions.configuration_failures,
        candidate.dispositions.behavioral_runs,
        candidate.behavioral.passed_scenarios,
        candidate.behavioral.macro_pass_rate * 100.0,
        candidate.dispositions.infrastructure_failures,
        candidate.dispositions.configuration_failures,
    );
    if let Some(repeated) = repeated {
        output.push_str("\n## Repeated paired comparison\n\n");
        output.push_str(&format_repeated_comparison_markdown(repeated));
    } else {
        output.push_str(
            "\nNo paired behavioral comparison was available because one or both profiles had no paired behavioral runs.\n",
        );
    }
    output
}

fn run_behavior_once(args: BehaviorRunArgs) -> Result<String, String> {
    let mut scenarios = load_behavior_suite(&args.suite)?;
    if let Some(scenario_id) = args.scenario_id.as_deref() {
        if args.suite != "debugging-hard" {
            return Err(
                "--scenario-id is restricted to the debugging-hard diagnostic suite".into(),
            );
        }
        scenarios.retain(|scenario| scenario.id == scenario_id);
        if scenarios.is_empty() {
            return Err(format!(
                "scenario '{scenario_id}' was not found in suite '{}'",
                args.suite
            ));
        }
    }
    let profile = parse_profile(&args.profile)?;
    let artifact_parent = args
        .artifacts
        .unwrap_or_else(|| PathBuf::from("target/behavior-evals"));
    let run_id = run_id();
    let run_root = artifact_parent.join(&run_id);
    let config = resolve_runtime_config(
        args.provider.as_deref(),
        args.model.as_deref(),
        args.davinci_bin.as_deref(),
        profile,
        None,
        &args.permission_mode,
        &run_root,
    )?;
    let runs = execute_suite(&scenarios, &config, &run_root, profile.id(), 1)?;
    let summary = aggregate_runs(&runs);
    let report = BehaviorExecutionReport {
        run_id: run_id.clone(),
        profile: Some(profile.id().to_string()),
        baseline: Some(summary.clone()),
        candidate: None,
        repeated: None,
    };
    let dispositions = DispositionSummaryArtifact {
        baseline: summary.dispositions.clone(),
        candidate: None,
    };
    let markdown = suite_markdown(&run_id, &args.suite, profile.id(), &summary);
    write_run_artifacts(
        &ArtifactRoot::new(&run_root),
        &report,
        &dispositions,
        &markdown,
    )?;
    Ok(format!(
        "behavior artifacts persisted at {}",
        run_root.display()
    ))
}

fn run_behavior_ab(args: BehaviorAbArgs) -> Result<String, String> {
    if args.repeats == 0 {
        return Err("repeats must be greater than zero".into());
    }
    let scenarios = load_behavior_suite(&args.suite)?;
    let baseline_profile = parse_profile(&args.baseline_profile)?;
    let candidate_profile = parse_profile(&args.candidate_profile)?;
    let baseline_policy = parse_model_policy(args.baseline_model_policy.as_deref())?;
    let candidate_policy = parse_model_policy(args.candidate_model_policy.as_deref())?;
    if baseline_profile == candidate_profile
        && (baseline_policy.is_none()
            || candidate_policy.is_none()
            || baseline_policy == candidate_policy)
    {
        return Err(
            "same-profile A/B requires distinct explicit --baseline-model-policy and --candidate-model-policy values"
                .into(),
        );
    }

    let baseline_hash = stable_hash_for_ab_variant(
        baseline_profile,
        &args.provider,
        &args.model,
        baseline_policy,
    );
    let candidate_hash = stable_hash_for_ab_variant(
        candidate_profile,
        &args.provider,
        &args.model,
        candidate_policy,
    );
    if baseline_hash == candidate_hash {
        return Err("baseline and candidate stable prompt hashes must differ".into());
    }

    let baseline_label = ab_variant_label(baseline_profile, baseline_policy);
    let candidate_label = ab_variant_label(candidate_profile, candidate_policy);
    let run_id = run_id();
    let run_root = args.artifacts.join(&run_id);
    let baseline_config = resolve_runtime_config(
        Some(&args.provider),
        Some(&args.model),
        Some(args.davinci_bin.as_path()),
        baseline_profile,
        baseline_policy,
        &args.permission_mode,
        &run_root,
    )?;
    let candidate_config = resolve_runtime_config(
        Some(&args.provider),
        Some(&args.model),
        Some(args.davinci_bin.as_path()),
        candidate_profile,
        candidate_policy,
        &args.permission_mode,
        &run_root,
    )?;
    let baseline_runs = execute_suite(
        &scenarios,
        &baseline_config,
        &run_root,
        &format!("baseline-{}", baseline_profile.id()),
        args.repeats,
    )?;
    let candidate_runs = execute_suite(
        &scenarios,
        &candidate_config,
        &run_root,
        &format!("candidate-{}", candidate_profile.id()),
        args.repeats,
    )?;
    let baseline = aggregate_runs(&baseline_runs);
    let candidate = aggregate_runs(&candidate_runs);
    let (baseline_samples, candidate_samples) = paired_samples(&baseline_runs, &candidate_runs);
    let repeated = if baseline_samples.is_empty() {
        None
    } else {
        Some(compare_repeated_runs(
            &baseline_samples,
            &candidate_samples,
            0xDA7A,
            2_000,
        )?)
    };
    let report = BehaviorExecutionReport {
        run_id: run_id.clone(),
        profile: None,
        baseline: Some(baseline.clone()),
        candidate: Some(candidate.clone()),
        repeated: repeated.clone(),
    };
    let dispositions = DispositionSummaryArtifact {
        baseline: baseline.dispositions.clone(),
        candidate: Some(candidate.dispositions.clone()),
    };
    let suite_hash = hash_json(&scenarios)?;
    let manifest = EvalRunManifest {
        run_id: run_id.clone(),
        davinci_commit: git_revision(),
        runner_version: env!("CARGO_PKG_VERSION").to_string(),
        suite_hash,
        provider: args.provider,
        model: args.model,
        baseline_prompt_hash: baseline_hash,
        candidate_prompt_hash: candidate_hash,
        permission_mode: args.permission_mode,
        tool_surface_hash: tool_surface_hash()?,
        repeats: args.repeats,
    };
    let markdown = ab_markdown(
        &run_id,
        &args.suite,
        &baseline_label,
        &candidate_label,
        &baseline,
        &candidate,
        repeated.as_ref(),
    );
    let artifacts = ArtifactRoot::new(&run_root);
    artifacts.write_manifest(&manifest)?;
    write_run_artifacts(&artifacts, &report, &dispositions, &markdown)?;
    artifacts.validate_complete()?;
    Ok(format!(
        "behavior A/B artifacts persisted at {}",
        run_root.display()
    ))
}

fn validate_behavior_gate(artifacts: PathBuf) -> Result<String, String> {
    let root = ArtifactRoot::new(&artifacts);
    let manifest = root.validate_complete()?;
    let bytes = fs::read(artifacts.join("disposition-summary.json"))
        .map_err(|error| format!("failed to read disposition summary: {error}"))?;
    let summary: DispositionSummaryArtifact = serde_json::from_slice(&bytes)
        .map_err(|error| format!("failed to decode disposition summary: {error}"))?;
    davinci_evals::behavior::scheduled_infrastructure_gate(&summary.baseline)?;
    if let Some(candidate) = &summary.candidate {
        davinci_evals::behavior::scheduled_infrastructure_gate(candidate)?;
    }
    Ok(format!(
        "behavior gate passed for run {} ({})",
        manifest.run_id,
        artifacts.display()
    ))
}

fn external_allowed_environment() -> BTreeMap<String, String> {
    let mut allowed = BTreeMap::new();
    for name in [
        "PATH",
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "GEMINI_API_KEY",
        "GOOGLE_API_KEY",
        "ANTHROPIC_BASE_URL",
        "OPENAI_BASE_URL",
        "PI_OFFLINE",
        "PATHEXT",
        "SystemRoot",
        "WINDIR",
        "ComSpec",
        "TEMP",
        "TMP",
    ] {
        if let Ok(value) = std::env::var(name) {
            allowed.insert(name.to_string(), value);
        }
    }
    allowed
}

fn resolve_external_binary(path: PathBuf) -> PathBuf {
    if path.is_absolute() || path.components().count() == 1 {
        path
    } else {
        std::env::current_dir()
            .map(|current| current.join(&path))
            .unwrap_or(path)
    }
}

fn behavior_category(category: ClaudeChallengeCategory) -> BehaviorCategory {
    match category {
        ClaudeChallengeCategory::FrontendDistinctiveness => BehaviorCategory::FrontendCapability,
        ClaudeChallengeCategory::ExplorationBeforeEdit => BehaviorCategory::Exploration,
        ClaudeChallengeCategory::AdaptiveFeatureWorkflowEfficiency => BehaviorCategory::Planning,
        ClaudeChallengeCategory::ReviewSpecialization => BehaviorCategory::VerificationIntegrity,
        ClaudeChallengeCategory::TriggerPrecision => BehaviorCategory::Collaboration,
    }
}

fn challenge_scenario(challenge: &ClaudePublicChallenge) -> BehaviorScenario {
    BehaviorScenario {
        id: challenge.id.clone(),
        category: behavior_category(challenge.category),
        request: challenge.request.clone(),
        repo_fixture: challenge.repo_fixture.clone(),
        requirements: challenge.requirements.clone(),
        limits: challenge.limits.clone(),
        setup_commands: challenge.setup_commands.clone(),
        verification_commands: challenge.verification_commands.clone(),
    }
}

fn resolve_davinci_harness(
    binary: Option<PathBuf>,
    provider: Option<&str>,
    model: Option<&str>,
) -> Result<(CommandHarness, String, String), String> {
    let provider = first_nonempty_value(provider, &["DAVINCI_PROVIDER", "PI_PROVIDER"])
        .ok_or_else(|| {
            "provider is required (--provider or DAVINCI_PROVIDER/PI_PROVIDER)".to_string()
        })?;
    let model = first_nonempty_value(model, &["DAVINCI_MODEL", "PI_MODEL"])
        .ok_or_else(|| "model is required (--model or DAVINCI_MODEL/PI_MODEL)".to_string())?;
    let binary = binary
        .or_else(|| std::env::var_os("DAVINCI_BIN").map(PathBuf::from))
        .unwrap_or_else(|| {
            if cfg!(windows) {
                PathBuf::from("target/release/davinci.exe")
            } else {
                PathBuf::from("target/release/davinci")
            }
        });
    let binary = resolve_external_binary(binary);
    let args = vec![
        "--mode".into(),
        "json".into(),
        "--prompt-profile".into(),
        "stable".into(),
        "--permission-mode".into(),
        "ask".into(),
        "--provider".into(),
        provider.clone(),
        "--model".into(),
        model.clone(),
        "-p".into(),
    ];
    Ok((
        CommandHarness::new("davinci", binary, args),
        provider,
        model,
    ))
}

fn external_run_passed(
    run: &ExternalRun,
    verification: &[davinci_evals::behavior::VerificationResult],
) -> bool {
    run.exit_code == 0 && verification.iter().all(|result| result.passed)
}

fn changed_path_is_allowed(path: &str, expected: &str) -> bool {
    path == expected || path.starts_with(&format!("{expected}/"))
}

fn has_unrelated_edit(run: &ExternalRun, scenario: &BehaviorScenario) -> bool {
    let allowed: Vec<&str> = scenario
        .requirements
        .iter()
        .filter_map(|requirement| match requirement {
            BehaviorRequirement::FileChanged { path } => Some(path.as_str()),
            _ => None,
        })
        .collect();
    run.files_changed.iter().any(|path| {
        !allowed
            .iter()
            .any(|expected| changed_path_is_allowed(path, expected))
    })
}

fn rate(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn median(values: &mut [u64]) -> f64 {
    values.sort_unstable();
    match values.len() {
        0 => 0.0,
        length if length % 2 == 1 => values[length / 2] as f64,
        length => (values[length / 2 - 1] as f64 + values[length / 2] as f64) / 2.0,
    }
}

fn competitor_metadata(raw: &CompetitorRawArtifact) -> FairComparisonMetadata {
    let fixture_snapshots: Vec<&str> = raw
        .samples
        .iter()
        .filter_map(|sample| sample.matched.as_ref())
        .map(|sample| sample.snapshot_hash.as_str())
        .collect();
    let repo_snapshot_hash = hash_json(&fixture_snapshots).unwrap_or_else(|_| "unavailable".into());
    FairComparisonMetadata {
        davinci_version: env!("CARGO_PKG_VERSION").to_string(),
        competitor_name: "claude-code".into(),
        competitor_version: raw.capabilities.version.clone(),
        davinci_model: raw.model.clone(),
        competitor_model: raw.competitor_model.clone(),
        models_controlled: raw.models_controlled,
        permission_mode: raw.permission_mode.clone(),
        timeout_seconds: raw.timeout_seconds,
        repo_snapshot_hash,
    }
}

fn competitor_statistics(
    raw: &CompetitorRawArtifact,
    challenges: &[ClaudePublicChallenge],
    metadata: &FairComparisonMetadata,
) -> Result<(CompetitorSuiteSummary, u32), String> {
    let scenarios: BTreeMap<String, BehaviorScenario> = challenges
        .iter()
        .map(|challenge| (challenge.id.clone(), challenge_scenario(challenge)))
        .collect();
    let mut matched = Vec::new();
    let mut scenario_repetitions: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
    let mut davinci_samples = Vec::new();
    let mut competitor_samples = Vec::new();
    let mut davinci_unrelated = 0;
    let mut competitor_unrelated = 0;
    let mut davinci_wall = Vec::new();
    let mut competitor_wall = Vec::new();

    for sample in &raw.samples {
        let Some(result) = &sample.matched else {
            continue;
        };
        let scenario = scenarios.get(&result.scenario_id).ok_or_else(|| {
            format!(
                "matched result references unknown scenario {}",
                result.scenario_id
            )
        })?;
        matched.push(result);
        scenario_repetitions
            .entry(result.scenario_id.clone())
            .or_default()
            .insert(sample.repetition);
        let davinci_passed = external_run_passed(&result.davinci, &result.davinci_verification);
        let competitor_passed =
            external_run_passed(&result.competitor, &result.competitor_verification);
        davinci_samples.push(ScenarioRunSample {
            scenario_id: result.scenario_id.clone(),
            repetition: sample.repetition,
            passed: davinci_passed,
            wall_ms: result.davinci.wall_ms,
            tool_calls: 0,
        });
        competitor_samples.push(ScenarioRunSample {
            scenario_id: result.scenario_id.clone(),
            repetition: sample.repetition,
            passed: competitor_passed,
            wall_ms: result.competitor.wall_ms,
            tool_calls: 0,
        });
        davinci_unrelated += usize::from(has_unrelated_edit(&result.davinci, scenario));
        competitor_unrelated += usize::from(has_unrelated_edit(&result.competitor, scenario));
        davinci_wall.push(result.davinci.wall_ms);
        competitor_wall.push(result.competitor.wall_ms);
    }

    let observations = pair_by_scenario_repetition(&competitor_samples, &davinci_samples)?;
    let ci = bootstrap_pass_delta_ci95(&observations, 0xC1A0, 2_000);
    let shared_scenarios = scenario_repetitions
        .values()
        .filter(|repetitions| {
            repetitions.len() == raw.repeats as usize
                && (0..raw.repeats).all(|repetition| repetitions.contains(&repetition))
        })
        .count();
    let denominator = matched.len();
    let davinci_passes = davinci_samples
        .iter()
        .filter(|sample| sample.passed)
        .count();
    let competitor_passes = competitor_samples
        .iter()
        .filter(|sample| sample.passed)
        .count();
    let summary = CompetitorSuiteSummary {
        comparison_class: comparison_class_for_metadata(metadata, !matched.is_empty()),
        shared_scenarios,
        davinci_pass_rate: rate(davinci_passes, denominator),
        competitor_pass_rate: rate(competitor_passes, denominator),
        pass_rate_delta_ci95: ci,
        davinci_unrelated_edit_rate: rate(davinci_unrelated, denominator),
        competitor_unrelated_edit_rate: rate(competitor_unrelated, denominator),
        davinci_wall_median_ms: median(&mut davinci_wall),
        competitor_wall_median_ms: median(&mut competitor_wall),
    };
    Ok((
        summary,
        raw.samples
            .iter()
            .filter(|sample| sample.matched.is_none())
            .count() as u32,
    ))
}

fn run_competitor_suite(args: CompetitorRunArgs) -> Result<String, String> {
    if args.suite != "claude-public-challenges" {
        return Err(format!(
            "unknown competitor suite '{}'; valid suite: claude-public-challenges",
            args.suite
        ));
    }
    if args.repeats == 0 || args.timeout_seconds == 0 {
        return Err("repeats and timeout_seconds must be greater than zero".into());
    }
    if args.artifacts.exists()
        && fs::read_dir(&args.artifacts)
            .map_err(|error| format!("failed to inspect artifact root: {error}"))?
            .next()
            .is_some()
    {
        return Err(format!(
            "competitor artifact root is not empty: {}",
            args.artifacts.display()
        ));
    }
    let challenges = load_claude_public_challenges()?;
    if challenges
        .iter()
        .any(|challenge| !challenge.setup_commands.is_empty())
    {
        return Err(
            "competitor challenge setup commands are not supported by the matched runner".into(),
        );
    }
    let suite_hash = hash_json(&challenges)?;
    let competitor_model =
        first_nonempty_value(None, &["COMPETITOR_MODEL"]).unwrap_or_else(|| "unknown".into());
    let requested_models_controlled =
        first_nonempty_value(None, &["DAVINCI_COMPETITOR_MODELS_CONTROLLED"])
            .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE"))
            .unwrap_or(false);
    let competitor = ClaudeCodeHarness::with_binary_and_options(
        resolve_external_binary(args.binary.clone()),
        Some(competitor_model.clone()),
        Some("ask".into()),
    );
    let capabilities = competitor.capabilities()?;
    if capabilities.version.is_none() {
        return Err("competitor version is not identifiable; refusing to run differential".into());
    }
    let models_controlled = requested_models_controlled
        && capabilities.supports_model_flag
        && capabilities.supports_permission_mode;
    let (davinci, provider, model) = resolve_davinci_harness(
        args.davinci_bin.clone(),
        args.provider.as_deref(),
        args.model.as_deref(),
    )?;
    let environment = external_allowed_environment();
    let mut samples = Vec::with_capacity(challenges.len() * args.repeats as usize);
    for repetition in 0..args.repeats {
        for challenge in &challenges {
            let scenario = challenge_scenario(challenge);
            let config = MatchedRunConfig {
                artifact_root: args
                    .artifacts
                    .join("scenarios")
                    .join(format!("repeat-{repetition}")),
                timeout: Duration::from_secs(args.timeout_seconds),
                environment: environment.clone(),
                ignore_paths: Vec::new(),
                verification_commands: scenario.verification_commands.clone(),
                source_fixture: None,
            };
            let result = davinci_evals::competitor::execute_matched_run(
                &scenario,
                &davinci,
                &competitor,
                &config,
            );
            match result {
                Ok(matched) => samples.push(CompetitorSampleArtifact {
                    scenario_id: challenge.id.clone(),
                    repetition,
                    matched: Some(matched),
                    error: None,
                }),
                Err(error) => samples.push(CompetitorSampleArtifact {
                    scenario_id: challenge.id.clone(),
                    repetition,
                    matched: None,
                    error: Some(error),
                }),
            }
        }
    }
    let raw = CompetitorRawArtifact {
        suite: args.suite,
        suite_hash,
        repeats: args.repeats,
        provider,
        model,
        competitor_binary: args.binary.display().to_string(),
        competitor_model,
        models_controlled,
        permission_mode: "ask".into(),
        timeout_seconds: args.timeout_seconds,
        capabilities,
        samples,
    };
    let artifacts = ArtifactRoot::new(args.artifacts.clone());
    artifacts.write_json("matched-runs.json", &raw)?;
    artifacts.write_json("run-metadata.json", &raw)?;
    artifacts.finalize_hashes()?;
    Ok(format!(
        "competitor matched artifacts persisted at {}",
        args.artifacts.display()
    ))
}

fn run_competitor_compare(args: CompetitorCompareArgs) -> Result<String, String> {
    if args.suite != "claude-public-challenges" {
        return Err(format!(
            "unknown competitor suite '{}'; valid suite: claude-public-challenges",
            args.suite
        ));
    }
    if args.repeats == 0 {
        return Err("repeats must be greater than zero".into());
    }
    let raw_bytes = fs::read(args.artifacts.join("matched-runs.json"))
        .map_err(|error| format!("failed to read matched-runs.json: {error}"))?;
    let raw: CompetitorRawArtifact = serde_json::from_slice(&raw_bytes)
        .map_err(|error| format!("failed to decode matched-runs.json: {error}"))?;
    if raw.suite != args.suite {
        return Err(format!(
            "artifact suite '{}' does not match requested suite '{}'",
            raw.suite, args.suite
        ));
    }
    if raw.repeats != args.repeats {
        return Err(format!(
            "artifact repetitions {} do not match requested {}",
            raw.repeats, args.repeats
        ));
    }
    let challenges = load_claude_public_challenges()?;
    let suite_hash = hash_json(&challenges)?;
    if suite_hash != raw.suite_hash {
        return Err("challenge suite changed since matched execution".into());
    }
    let metadata = competitor_metadata(&raw);
    let (summary, runtime_boundary_failures) = competitor_statistics(&raw, &challenges, &metadata)?;
    let claim_failures = evaluate_competitor_claim(
        &summary,
        CompetitorClaimInputs {
            repetitions: raw.repeats,
            runtime_boundary_failures,
            approved_non_inferiority: false,
        },
    )
    .err()
    .unwrap_or_default();
    let claim_failure_text: Vec<String> = claim_failures
        .iter()
        .map(|failure| failure.reason.clone())
        .collect();

    let mut markdown = format!(
        "# DaVinci versus Claude Code differential\n\n- Suite: `{}`\n- Repetitions: `{}`\n- Runtime-boundary failures: `{}`\n\n## Suite summary\n\n{}",
        raw.suite,
        raw.repeats,
        runtime_boundary_failures,
        format_competitor_suite_summary(&summary),
    );
    markdown.push_str("\n## Fair-comparison metadata\n\n");
    markdown.push_str(&format!(
        "- DaVinci version: `{}`\n- Claude Code version: `{}`\n- DaVinci model: `{}`\n- Competitor model: `{}`\n- Models controlled: `{}`\n- Permission mode: `{}`\n- Suite snapshot: `{}`\n",
        metadata.davinci_version,
        metadata.competitor_version.as_deref().unwrap_or("unknown"),
        metadata.davinci_model,
        metadata.competitor_model,
        metadata.models_controlled,
        metadata.permission_mode,
        metadata.repo_snapshot_hash,
    ));
    if claim_failure_text.is_empty() {
        markdown.push_str("\n## Claim gate\n\nPASS\n");
    } else {
        markdown.push_str("\n## Claim gate\n\nFAIL (claim is not eligible)\n\n");
        for failure in &claim_failure_text {
            markdown.push_str(&format!("- {failure}\n"));
        }
    }
    if let Some(sample) = raw
        .samples
        .iter()
        .find_map(|sample| sample.matched.as_ref())
    {
        if let Some(challenge) = challenges
            .iter()
            .find(|challenge| challenge.id == sample.scenario_id)
        {
            markdown.push('\n');
            markdown.push_str(&format_competitor_report_markdown(
                &CompetitorComparisonReport {
                    metadata: metadata.clone(),
                    task_id: sample.scenario_id.clone(),
                    request: challenge.request.clone(),
                    davinci_passed: external_run_passed(
                        &sample.davinci,
                        &sample.davinci_verification,
                    ),
                    competitor_passed: external_run_passed(
                        &sample.competitor,
                        &sample.competitor_verification,
                    ),
                    davinci_wall_ms: sample.davinci.wall_ms,
                    competitor_wall_ms: sample.competitor.wall_ms,
                    davinci_files_changed: sample.davinci.files_changed.clone(),
                    competitor_files_changed: sample.competitor.files_changed.clone(),
                },
            ));
        }
    }

    let comparison = CompetitorComparisonArtifact {
        metadata,
        summary,
        runtime_boundary_failures,
        claim_failures: claim_failure_text.clone(),
        samples: raw.samples,
    };
    let artifacts = ArtifactRoot::new(args.artifacts.clone());
    artifacts.write_json("comparison.json", &comparison)?;
    artifacts.write_json("summary.json", &comparison.summary)?;
    artifacts.write_text("summary.md", &markdown)?;
    artifacts.finalize_hashes()?;
    if claim_failure_text.is_empty() {
        Ok(format!(
            "competitor comparison passed at {}",
            args.artifacts.display()
        ))
    } else {
        Err(format!(
            "competitor claim gate failed; report persisted at {}: {}",
            args.artifacts.display(),
            claim_failure_text.join("; ")
        ))
    }
}

fn dispatch(command: TopLevelCommand) -> Result<String, String> {
    match command {
        TopLevelCommand::Corpus {
            command: CorpusCommand::Verify,
        } => {
            let scenarios = davinci_evals::behavior::load_core_200_corpus()?;
            Ok(format!("verified {} behavior scenarios", scenarios.len()))
        }
        TopLevelCommand::Behavior { command } => match command {
            BehaviorCommand::Run(args) => run_behavior_once(args),
            BehaviorCommand::Ab(args) => run_behavior_ab(args),
            BehaviorCommand::Gate(args) => validate_behavior_gate(args.artifacts),
            BehaviorCommand::PromoteCheck(args) => {
                let bytes = fs::read(&args.evidence).map_err(|error| {
                    format!(
                        "failed to read promotion evidence {}: {error}",
                        args.evidence.display()
                    )
                })?;
                let evidence: PromptPromotionEvidence = serde_json::from_slice(&bytes)
                    .map_err(|error| format!("failed to decode promotion evidence: {error}"))?;
                validate_promotion_evidence_against_hashes(
                    &evidence,
                    &args.artifacts_root,
                    args.candidate_prompt_hash.as_deref(),
                    args.stable_prompt_hash.as_deref(),
                )
                .map_err(|failures| {
                    let reasons = failures.join("; ");
                    format!("promotion evidence invalid: {reasons}")
                })?;
                Ok(format!(
                    "promotion evidence eligible: {}",
                    evidence.candidate_id
                ))
            }
        },
        TopLevelCommand::Competitor { command } => match command {
            CompetitorCommand::Run(args) => run_competitor_suite(args),
            CompetitorCommand::Compare(args) => run_competitor_compare(args),
        },
    }
}

fn main() {
    if let Err(error) = dispatch(Cli::parse().command) {
        eprintln!("error: {error}");
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(arguments: &[&str]) -> Cli {
        Cli::try_parse_from(arguments).unwrap()
    }

    #[test]
    fn parses_corpus_verify() {
        assert!(matches!(
            parse(&["davinci-evals", "corpus", "verify"]).command,
            TopLevelCommand::Corpus {
                command: CorpusCommand::Verify
            }
        ));
    }

    #[test]
    fn parses_behavior_commands() {
        assert!(matches!(
            parse(&["davinci-evals", "behavior", "run"]).command,
            TopLevelCommand::Behavior {
                command: BehaviorCommand::Run(_)
            }
        ));
        assert!(matches!(
            parse(&[
                "davinci-evals",
                "behavior",
                "ab",
                "--provider",
                "openai-codex",
                "--model",
                "gpt-5.6-sol",
                "--davinci-bin",
                "target/debug/davinci.exe"
            ])
            .command,
            TopLevelCommand::Behavior {
                command: BehaviorCommand::Ab(_)
            }
        ));
        assert!(matches!(
            parse(&["davinci-evals", "behavior", "gate", "--artifacts", "out"]).command,
            TopLevelCommand::Behavior {
                command: BehaviorCommand::Gate(_)
            }
        ));
        assert!(matches!(
            parse(&[
                "davinci-evals",
                "behavior",
                "promote-check",
                "--evidence",
                "evidence.json"
            ])
            .command,
            TopLevelCommand::Behavior {
                command: BehaviorCommand::PromoteCheck(_)
            }
        ));
    }

    #[test]
    fn parses_bounded_behavior_scenario_probe() {
        let cli = parse(&[
            "davinci-evals",
            "behavior",
            "run",
            "--suite",
            "debugging-hard",
            "--scenario-id",
            "debugging-hard-001",
        ]);
        let TopLevelCommand::Behavior {
            command: BehaviorCommand::Run(args),
        } = cli.command
        else {
            panic!("expected behavior run");
        };
        assert_eq!(args.scenario_id.as_deref(), Some("debugging-hard-001"));
    }

    #[test]
    fn parses_competitor_commands() {
        assert!(matches!(
            parse(&["davinci-evals", "competitor", "run", "--binary", "claude"]).command,
            TopLevelCommand::Competitor {
                command: CompetitorCommand::Run(_)
            }
        ));
        assert!(matches!(
            parse(&[
                "davinci-evals",
                "competitor",
                "compare",
                "--artifacts",
                "out"
            ])
            .command,
            TopLevelCommand::Competitor {
                command: CompetitorCommand::Compare(_)
            }
        ));
    }

    #[test]
    fn parses_same_profile_model_policy_ab_overrides() {
        let cli = parse(&[
            "davinci-evals",
            "behavior",
            "ab",
            "--provider",
            "openai-codex",
            "--model",
            "gpt-6-astra",
            "--davinci-bin",
            "davinci",
            "--baseline-profile",
            "stable",
            "--candidate-profile",
            "stable",
            "--baseline-model-policy",
            "default",
            "--candidate-model-policy",
            "gpt6-astra",
        ]);
        let TopLevelCommand::Behavior {
            command: BehaviorCommand::Ab(args),
        } = cli.command
        else {
            panic!("expected behavior ab");
        };
        assert_eq!(args.baseline_model_policy.as_deref(), Some("default"));
        assert_eq!(args.candidate_model_policy.as_deref(), Some("gpt6-astra"));
    }

    #[test]
    fn same_profile_model_policies_have_distinct_stable_hashes() {
        let default_hash = stable_hash_for_ab_variant(
            PromptProfile::Stable,
            "openai-codex",
            "gpt-6-astra",
            Some(davinci_agent::prompt::PromptModelPolicy::Default),
        );
        let astra_hash = stable_hash_for_ab_variant(
            PromptProfile::Stable,
            "openai-codex",
            "gpt-6-astra",
            Some(davinci_agent::prompt::PromptModelPolicy::Gpt6Astra),
        );
        assert_ne!(default_hash, astra_hash);
    }

    #[test]
    fn gpt6_astra_suite_is_available_to_ab_runner() {
        let scenarios = load_behavior_suite("gpt6-astra").expect("Astra regression suite");
        assert_eq!(scenarios.len(), 4);
        assert!(scenarios
            .iter()
            .all(|scenario| scenario.id.starts_with("astra-")));
    }
    #[test]
    fn promotion_repeat_default_is_three() {
        let cli = parse(&[
            "davinci-evals",
            "behavior",
            "ab",
            "--provider",
            "fixture",
            "--model",
            "model",
            "--davinci-bin",
            "davinci",
        ]);
        let TopLevelCommand::Behavior {
            command: BehaviorCommand::Ab(args),
        } = cli.command
        else {
            panic!("expected behavior ab");
        };
        assert_eq!(args.repeats, DEFAULT_PROMOTION_REPEATS);
    }

    #[test]
    fn corpus_dispatch_is_offline_and_deterministic() {
        let output = dispatch(TopLevelCommand::Corpus {
            command: CorpusCommand::Verify,
        })
        .unwrap();
        assert_eq!(output, "verified 200 behavior scenarios");
    }
}
