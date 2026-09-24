//! `/memory-page`: writes a self-contained HTML page that shows whether
//! vector memory is connected and working, then opens it in the browser.
//!
//! No TypeScript counterpart. Every number on the page comes from the live
//! `VectorMemory` store and from a probe of the configured Ollama server made
//! when the command runs; the page is a snapshot, and running the command
//! again refreshes it.

use super::graph::store::iso8601_utc;
use super::vector_memory::{MemoryHit, MemoryKind, MemoryRecord, VectorMemory};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const PAGE_FILE: &str = "memory-page.html";
const RECENT_RECORDS: usize = 40;
const TEXT_PREVIEW_CHARS: usize = 400;
const QUERY_PREVIEW_CHARS: usize = 160;
const PROBE_TEXT: &str = "davinci memory page connectivity probe";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageOptions {
    pub query: Option<String>,
    pub open: bool,
}

/// `/memory-page [--no-open] [query]`.
pub fn parse_args(args: &str) -> PageOptions {
    let mut open = true;
    let mut words = Vec::new();
    for word in args.split_whitespace() {
        if word == "--no-open" {
            open = false;
        } else {
            words.push(word);
        }
    }
    let query = words.join(" ");
    PageOptions {
        query: (!query.is_empty()).then_some(query),
        open,
    }
}

/// What the embedding server answered when the page was built.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    pub attempted: bool,
    pub reachable: bool,
    pub server_error: Option<String>,
    pub installed_models: Vec<String>,
    pub model_installed: Option<bool>,
    pub embed_ok: bool,
    pub embed_dimensions: Option<usize>,
    pub embed_ms: Option<u64>,
    pub embed_error: Option<String>,
}

fn model_matches(configured: &str, installed: &str) -> bool {
    installed == configured
        || (!configured.contains(':')
            && installed.split(':').next() == Some(configured)
            && installed.ends_with(":latest"))
}

pub fn probe(memory: &VectorMemory) -> Probe {
    let mut probe = Probe::default();
    if !memory.config.enabled {
        return probe;
    }
    probe.attempted = true;
    let base = memory.config.ollama_url.trim_end_matches('/');
    let timeout = Duration::from_secs(memory.config.request_timeout_seconds.clamp(1, 10));
    let agent = ureq::AgentBuilder::new().timeout(timeout).build();
    match agent
        .get(&format!("{base}/api/tags"))
        .call()
        .map_err(|error| error.to_string())
        .and_then(|response| {
            response
                .into_json::<Value>()
                .map_err(|error| error.to_string())
        }) {
        Ok(tags) => {
            probe.reachable = true;
            probe.installed_models = tags["models"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|model| model["name"].as_str().map(str::to_string))
                .collect();
            probe.model_installed = Some(
                probe
                    .installed_models
                    .iter()
                    .any(|name| model_matches(&memory.config.embedding_model, name)),
            );
        }
        Err(error) => {
            probe.server_error = Some(error);
            return probe;
        }
    }
    let started = Instant::now();
    match memory.embed_query_text(PROBE_TEXT) {
        Ok(vector) => {
            probe.embed_ok = true;
            probe.embed_dimensions = Some(vector.len());
            probe.embed_ms = Some(started.elapsed().as_millis() as u64);
        }
        Err(error) => probe.embed_error = Some(error.to_string()),
    }
    probe
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    /// Ollama answers, the model embeds, and every record has a vector.
    Connected,
    /// Memory works, but only part of it: lexical search only, or records
    /// dense search cannot find.
    Degraded,
    Disabled,
}

impl Verdict {
    fn label(self) -> &'static str {
        match self {
            Verdict::Connected => "Connected",
            Verdict::Degraded => "Degraded",
            Verdict::Disabled => "Disabled",
        }
    }
}

/// Everything the page shows, gathered once so the HTML and the command's
/// JSON answer cannot disagree.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub generated_at: u64,
    pub verdict: Verdict,
    pub warnings: Vec<String>,
    pub probe: Probe,
    pub status: Value,
    pub legacy_records: Option<(PathBuf, usize)>,
    pub duplicate_ids: usize,
    pub query: Option<String>,
    #[serde(skip)]
    pub hits: Vec<MemoryHit>,
    #[serde(skip)]
    pub recent: Vec<MemoryRecord>,
}

/// Records in `.pi/vector-memory` that are no longer read because a
/// `.davinci/vector-memory` store exists beside them.
fn shadowed_legacy_store(memory: &VectorMemory) -> Option<(PathBuf, usize)> {
    let active = memory.local_path();
    let legacy = memory
        .cwd
        .join(".pi")
        .join("vector-memory")
        .join("records.jsonl");
    if active == legacy || !legacy.is_file() {
        return None;
    }
    let count = std::fs::read_to_string(&legacy)
        .ok()?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    (count > 0).then_some((legacy, count))
}

pub fn build_report(memory: &VectorMemory, options: &PageOptions) -> Report {
    let status = memory.status();
    let probe = probe(memory);
    let records = memory.records();
    let mut seen = std::collections::HashSet::new();
    let duplicate_ids = records
        .iter()
        .filter(|record| !seen.insert(record.id.as_str()))
        .count();
    let query = options.query.clone().or_else(|| {
        records
            .iter()
            .rev()
            .find(|record| record.kind == MemoryKind::Task)
            .map(|record| record.text.chars().take(QUERY_PREVIEW_CHARS).collect())
    });
    let hits = query
        .as_deref()
        .map(|query| memory.search(query, memory.config.result_limit))
        .unwrap_or_default();
    let mut recent = records.to_vec();
    recent.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    recent.truncate(RECENT_RECORDS);
    for record in &mut recent {
        record.embedding = record.embedding.as_ref().map(|_| Vec::new());
    }

    let lag = status["projectionLag"].as_u64().unwrap_or(0);
    let legacy_records = shadowed_legacy_store(memory);
    let mut warnings = Vec::new();
    if !memory.config.enabled {
        warnings
            .push("Vector memory is disabled (`enabled: false` or PI_MEMORY_ENABLED=0).".into());
    }
    if probe.attempted && !probe.reachable {
        warnings.push(format!(
            "Ollama at {} did not answer: {}. Search falls back to keywords only.",
            memory.config.ollama_url,
            probe.server_error.as_deref().unwrap_or("unknown error")
        ));
    }
    if probe.model_installed == Some(false) {
        warnings.push(format!(
            "Embedding model `{}` is not installed. Run `ollama pull {}`.",
            memory.config.embedding_model, memory.config.embedding_model
        ));
    }
    if probe.reachable && !probe.embed_ok {
        warnings.push(format!(
            "The test embedding failed: {}",
            probe.embed_error.as_deref().unwrap_or("unknown error")
        ));
    }
    if let Some(dimensions) = probe.embed_dimensions {
        if dimensions != memory.config.embedding_dimensions {
            warnings.push(format!(
                "The model returned {dimensions} dimensions; memory is configured for {}.",
                memory.config.embedding_dimensions
            ));
        }
    }
    if lag > 0 {
        warnings.push(format!(
            "{lag} record(s) have no current embedding, so semantic search cannot find them. Run /memory-reindex."
        ));
    }
    if !memory.config.automatic_retrieval {
        warnings.push(
            "Automatic retrieval is off: memories are only used through /memory-search.".into(),
        );
    }
    if let Some((path, count)) = &legacy_records {
        warnings.push(format!(
            "{count} record(s) in the legacy store {} are not read, because the .davinci store takes precedence.",
            path.display()
        ));
    }
    if duplicate_ids > 0 {
        warnings.push(format!(
            "{duplicate_ids} record(s) share an id with another record (written before the id fix). Run /memory-reindex."
        ));
    }

    let dense_ready = probe.reachable && probe.model_installed != Some(false) && probe.embed_ok;
    let verdict = if !memory.config.enabled {
        Verdict::Disabled
    } else if dense_ready && lag == 0 {
        Verdict::Connected
    } else {
        Verdict::Degraded
    };
    Report {
        generated_at: davinci_session::now_ms(),
        verdict,
        warnings,
        probe,
        status,
        legacy_records,
        duplicate_ids,
        query,
        hits,
        recent,
    }
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(character),
        }
    }
    out
}

fn preview(text: &str, limit: usize) -> String {
    let mut preview: String = text.chars().take(limit).collect();
    if text.chars().count() > limit {
        preview.push('…');
    }
    preview
}

fn kind_name(kind: MemoryKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{kind:?}"))
}

fn check(ok: bool, label: &str, detail: &str) -> String {
    format!(
        "<div class=\"check {}\"><span class=\"dot\" aria-hidden=\"true\"></span><div><div class=\"check-label\">{}</div><div class=\"check-detail\">{}</div></div></div>",
        if ok { "ok" } else { "bad" },
        escape(label),
        escape(detail)
    )
}

fn fact(label: &str, value: impl std::fmt::Display) -> String {
    format!(
        "<div class=\"fact\"><dt>{}</dt><dd>{}</dd></div>",
        escape(label),
        escape(&value.to_string())
    )
}

const STYLE: &str = r#"
:root { --bg:#faf9f7; --panel:#ffffff; --text:#1f1d1a; --muted:#6b665e; --line:#e7e3dc;
  --accent:#d97757; --ok:#2f7d4f; --warn:#b7791f; --bad:#b83a3a; --chip:#f1eee8; }
@media (prefers-color-scheme: dark) { :root:not([data-theme="light"]) {
  --bg:#181715; --panel:#211f1c; --text:#ece8e1; --muted:#a39d93; --line:#34302b;
  --accent:#e08a6c; --ok:#5fb884; --warn:#d9a441; --bad:#e06c6c; --chip:#2b2824; } }
* { box-sizing:border-box; }
body { margin:0; background:var(--bg); color:var(--text);
  font:15px/1.5 -apple-system, "Segoe UI", system-ui, sans-serif; }
main { max-width:1080px; margin:0 auto; padding:32px 16px 64px; }
header { display:flex; flex-wrap:wrap; gap:12px 24px; align-items:baseline; justify-content:space-between; }
h1 { font-size:24px; margin:0; }
h2 { font-size:16px; margin:32px 0 12px; }
.meta { color:var(--muted); font-size:13px; overflow-wrap:anywhere; }
.badge { display:inline-block; padding:4px 12px; border-radius:999px; font-weight:600; font-size:14px; }
.badge.connected { background:color-mix(in srgb, var(--ok) 18%, transparent); color:var(--ok); }
.badge.degraded { background:color-mix(in srgb, var(--warn) 18%, transparent); color:var(--warn); }
.badge.disabled { background:var(--chip); color:var(--muted); }
.grid { display:grid; grid-template-columns:repeat(auto-fit, minmax(230px, 1fr)); gap:12px; }
.check { display:flex; gap:10px; align-items:flex-start; background:var(--panel); border:1px solid var(--line); border-radius:10px; padding:12px; }
.dot { width:10px; height:10px; border-radius:50%; margin-top:6px; flex:none; }
.check.ok .dot { background:var(--ok); } .check.bad .dot { background:var(--bad); }
.check-label { font-weight:600; } .check-detail { color:var(--muted); font-size:13px; overflow-wrap:anywhere; }
.warnings { list-style:none; padding:0; margin:16px 0 0; display:grid; gap:8px; }
.warnings li { border-left:3px solid var(--warn); background:var(--panel); padding:8px 12px; border-radius:6px; }
dl { display:grid; grid-template-columns:repeat(auto-fit, minmax(200px, 1fr)); gap:8px; margin:0; }
.fact { background:var(--panel); border:1px solid var(--line); border-radius:8px; padding:8px 12px; }
.fact dt { color:var(--muted); font-size:12px; } .fact dd { margin:0; font-weight:600; overflow-wrap:anywhere; }
.kinds { display:flex; flex-wrap:wrap; gap:6px; }
.chip { background:var(--chip); border-radius:999px; padding:2px 10px; font-size:13px; }
.table-wrap { overflow-x:auto; border:1px solid var(--line); border-radius:10px; background:var(--panel); }
table { border-collapse:collapse; width:100%; font-size:13px; }
th, td { text-align:left; padding:8px 10px; border-bottom:1px solid var(--line); vertical-align:top; }
th { color:var(--muted); font-weight:600; white-space:nowrap; }
tr:last-child td { border-bottom:none; }
td.text { min-width:280px; white-space:pre-wrap; overflow-wrap:anywhere; }
td.num { font-variant-numeric:tabular-nums; white-space:nowrap; }
.empty { color:var(--muted); padding:12px; }
code { background:var(--chip); padding:1px 5px; border-radius:4px; }
"#;

pub fn render_html(memory: &VectorMemory, report: &Report) -> String {
    let status = &report.status;
    let probe = &report.probe;
    let mut html = String::new();
    let _ = write!(
        html,
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>Davinci Memory</title><style>{STYLE}</style></head><body><main>"
    );
    let _ = write!(
        html,
        "<header><div><h1>Vector memory</h1><div class=\"meta\">{}</div><div class=\"meta\">Generated {} &middot; run <code>/memory-page</code> again to refresh</div></div><span class=\"badge {}\">{}</span></header>",
        escape(&memory.cwd.display().to_string()),
        escape(&iso8601_utc(report.generated_at)),
        match report.verdict {
            Verdict::Connected => "connected",
            Verdict::Degraded => "degraded",
            Verdict::Disabled => "disabled",
        },
        report.verdict.label()
    );
    if !report.warnings.is_empty() {
        html.push_str("<ul class=\"warnings\">");
        for warning in &report.warnings {
            let _ = write!(html, "<li>{}</li>", escape(warning));
        }
        html.push_str("</ul>");
    }

    html.push_str("<h2>Connection</h2><div class=\"grid\">");
    html.push_str(&check(
        probe.reachable,
        "Ollama server",
        &if probe.reachable {
            format!(
                "{} answered ({} models installed)",
                memory.config.ollama_url,
                probe.installed_models.len()
            )
        } else if probe.attempted {
            format!(
                "{}: {}",
                memory.config.ollama_url,
                probe.server_error.as_deref().unwrap_or("no answer")
            )
        } else {
            "not probed: memory is disabled".into()
        },
    ));
    html.push_str(&check(
        probe.model_installed == Some(true),
        "Embedding model",
        &match probe.model_installed {
            Some(true) => format!("{} is installed", memory.config.embedding_model),
            Some(false) => format!("{} is not installed", memory.config.embedding_model),
            None => format!("{} (not checked)", memory.config.embedding_model),
        },
    ));
    html.push_str(&check(
        probe.embed_ok && probe.embed_dimensions == Some(memory.config.embedding_dimensions),
        "Test embedding",
        &match (probe.embed_dimensions, probe.embed_ms) {
            (Some(dimensions), Some(ms)) => {
                format!(
                    "{dimensions} dimensions in {ms} ms (expected {})",
                    memory.config.embedding_dimensions
                )
            }
            _ => probe
                .embed_error
                .clone()
                .unwrap_or_else(|| "not attempted".into()),
        },
    ));
    let dense_hits = report.hits.iter().any(|hit| hit.dense_score > 0.0);
    html.push_str(&check(
        dense_hits,
        "Semantic search",
        if report.hits.is_empty() {
            "no retrieval check: the store has no matching records"
        } else if dense_hits {
            "the retrieval check below used vector similarity"
        } else {
            "the retrieval check below matched on keywords only"
        },
    ));
    html.push_str("</div>");

    html.push_str("<h2>Store</h2><dl>");
    html.push_str(&fact("Records", status["records"].as_u64().unwrap_or(0)));
    html.push_str(&fact(
        "With embeddings",
        status["embedded"].as_u64().unwrap_or(0),
    ));
    html.push_str(&fact(
        "Missing embeddings",
        status["projectionLag"].as_u64().unwrap_or(0),
    ));
    html.push_str(&fact(
        "Automatic retrieval",
        if memory.config.automatic_retrieval {
            "on"
        } else {
            "off"
        },
    ));
    html.push_str(&fact("File", memory.local_path().display()));
    html.push_str(&fact("Repository id", preview(&memory.repo_id, 16)));
    html.push_str("</dl>");
    let kinds = status["kinds"].as_object().cloned().unwrap_or_default();
    if !kinds.is_empty() {
        html.push_str("<div class=\"kinds\" style=\"margin-top:12px\">");
        let sorted: BTreeMap<_, _> = kinds.into_iter().collect();
        for (kind, count) in sorted {
            let _ = write!(
                html,
                "<span class=\"chip\">{} {}</span>",
                escape(&kind),
                count.as_u64().unwrap_or(0)
            );
        }
        html.push_str("</div>");
    }

    html.push_str("<h2>Retrieval check</h2>");
    match &report.query {
        Some(query) => {
            let _ = write!(
                html,
                "<p class=\"meta\">Query: <code>{}</code> &middot; the same search that runs before each prompt</p>",
                escape(query)
            );
            if report.hits.is_empty() {
                html.push_str("<div class=\"table-wrap\"><div class=\"empty\">No memory scored above the minimum score.</div></div>");
            } else {
                html.push_str("<div class=\"table-wrap\"><table><thead><tr><th>Score</th><th>Semantic</th><th>Keyword</th><th>Kind</th><th>Memory</th></tr></thead><tbody>");
                for hit in &report.hits {
                    let _ = write!(
                        html,
                        "<tr><td class=\"num\">{:.2}</td><td class=\"num\">{:.2}</td><td class=\"num\">{:.2}</td><td>{}</td><td class=\"text\">{}</td></tr>",
                        hit.score,
                        hit.dense_score,
                        hit.lexical_score,
                        escape(&kind_name(hit.record.kind)),
                        escape(&preview(&hit.record.text, TEXT_PREVIEW_CHARS))
                    );
                }
                html.push_str("</tbody></table></div>");
            }
        }
        None => html.push_str(
            "<div class=\"table-wrap\"><div class=\"empty\">No records yet. Memory is written after each turn; run a prompt, then this command again.</div></div>",
        ),
    }

    let _ = write!(
        html,
        "<h2>Recent memories <span class=\"meta\">(newest {} of {})</span></h2>",
        report.recent.len(),
        status["records"].as_u64().unwrap_or(0)
    );
    if report.recent.is_empty() {
        html.push_str(
            "<div class=\"table-wrap\"><div class=\"empty\">The store is empty.</div></div>",
        );
    } else {
        html.push_str("<div class=\"table-wrap\"><table><thead><tr><th>Written</th><th>Kind</th><th>Vector</th><th>Source</th><th>Memory</th></tr></thead><tbody>");
        for record in &report.recent {
            let _ = write!(
                html,
                "<tr><td class=\"num\">{}</td><td>{}</td><td>{}</td><td>{}</td><td class=\"text\">{}</td></tr>",
                escape(&iso8601_utc(record.created_at).replace('T', " ")),
                escape(&kind_name(record.kind)),
                if record.embedding.is_some() { "yes" } else { "no" },
                escape(&record.source),
                escape(&preview(&record.text, TEXT_PREVIEW_CHARS))
            );
        }
        html.push_str("</tbody></table></div>");
    }

    html.push_str("<h2>Configuration</h2><dl>");
    for key in [
        "embeddingModel",
        "embeddingDimensions",
        "ollama",
        "resultLimit",
        "candidateLimit",
        "minimumScore",
        "maxInjectedTokens",
        "maxIndexChunks",
        "projectionProfile",
    ] {
        let value = match &status[key] {
            Value::String(text) => text.clone(),
            Value::Number(number) => number
                .as_f64()
                .filter(|value| value.fract() != 0.0)
                .map(|value| format!("{value:.2}"))
                .unwrap_or_else(|| number.to_string()),
            other => other.to_string(),
        };
        html.push_str(&fact(key, value));
    }
    html.push_str("</dl></main></body></html>\n");
    html
}

/// Where the page is written: beside the records it describes.
pub fn page_path(memory: &VectorMemory) -> PathBuf {
    memory
        .local_path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| memory.cwd.join(".davinci").join("vector-memory"))
        .join(PAGE_FILE)
}

/// Build, write and (unless `--no-open`) open the page. The answer is the
/// report without its tables, so RPC and print mode can read the verdict.
pub fn command(memory: &VectorMemory, args: &str) -> Result<Value, String> {
    let options = parse_args(args);
    let report = build_report(memory, &options);
    let html = render_html(memory, &report);
    let path = page_path(memory);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(&path, html).map_err(|error| error.to_string())?;
    let opened = if options.open {
        Some(davinci_tui::open_browser(&path.display().to_string()))
    } else {
        None
    };
    Ok(json!({
        "path": path,
        "opened": opened.is_some(),
        "verdict": report.verdict,
        "warnings": report.warnings,
        "records": report.status["records"],
        "embedded": report.status["embedded"],
        "probe": report.probe,
        "retrievalQuery": report.query,
        "retrievalHits": report.hits.len(),
    }))
}

#[cfg(test)]
mod tests {
    use super::super::vector_memory::{MemoryMessage, VectorMemoryConfig};
    use super::*;

    fn closed_port_url() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        format!("http://{address}")
    }

    #[test]
    fn arguments_split_the_flag_from_the_query() {
        assert_eq!(
            parse_args("--no-open deploy region"),
            PageOptions {
                query: Some("deploy region".into()),
                open: false
            }
        );
        assert_eq!(
            parse_args(""),
            PageOptions {
                query: None,
                open: true
            }
        );
    }

    #[test]
    fn model_names_match_with_or_without_the_latest_tag() {
        assert!(model_matches("embeddinggemma", "embeddinggemma:latest"));
        assert!(model_matches("embeddinggemma:300m", "embeddinggemma:300m"));
        assert!(!model_matches("embeddinggemma", "embeddinggemma:300m"));
        assert!(!model_matches("nomic-embed-text", "embeddinggemma:latest"));
    }

    #[test]
    fn a_live_embedding_server_makes_the_page_connected() {
        let directory = tempfile::tempdir().unwrap();
        let config = VectorMemoryConfig {
            ollama_url: super::super::vector_memory::tests::fake_ollama(8),
            embedding_dimensions: 8,
            ..VectorMemoryConfig::default()
        };
        let mut memory = VectorMemory::with_config(directory.path().to_path_buf(), config);
        memory
            .index_messages(&[MemoryMessage {
                role: "user".into(),
                content: "Zephyr deploys with tools/zephyr-deploy.ps1".into(),
            }])
            .unwrap();
        let answer = command(&memory, "--no-open zephyr deploy").unwrap();
        assert_eq!(answer["verdict"], "connected", "{answer}");
        assert_eq!(answer["opened"], false);
        assert_eq!(answer["probe"]["embedDimensions"], 8);
        assert!(answer["retrievalHits"].as_u64().unwrap() > 0);
        let html = std::fs::read_to_string(page_path(&memory)).unwrap();
        assert!(html.contains("Connected"));
        assert!(html.contains("zephyr-deploy.ps1"));
        assert!(html.contains("vector similarity"));
    }

    #[test]
    fn an_unreachable_server_is_degraded_and_says_why() {
        let directory = tempfile::tempdir().unwrap();
        let config = VectorMemoryConfig {
            ollama_url: closed_port_url(),
            request_timeout_seconds: 2,
            ..VectorMemoryConfig::default()
        };
        let memory = VectorMemory::with_config(directory.path().to_path_buf(), config);
        let answer = command(&memory, "--no-open").unwrap();
        assert_eq!(answer["verdict"], "degraded");
        assert_eq!(answer["probe"]["reachable"], false);
        assert!(answer["warnings"][0]
            .as_str()
            .unwrap()
            .contains("did not answer"));
    }

    #[test]
    fn record_text_is_escaped_in_the_page() {
        let directory = tempfile::tempdir().unwrap();
        let config = VectorMemoryConfig {
            ollama_url: closed_port_url(),
            request_timeout_seconds: 2,
            ..VectorMemoryConfig::default()
        };
        let mut memory = VectorMemory::with_config(directory.path().to_path_buf(), config);
        memory.mark_dense_offline();
        memory
            .index_messages(&[MemoryMessage {
                role: "user".into(),
                content: "<script>alert('x')</script> & more".into(),
            }])
            .unwrap();
        command(&memory, "--no-open").unwrap();
        let html = std::fs::read_to_string(page_path(&memory)).unwrap();
        assert!(!html.contains("<script>alert"));
        assert!(html.contains("&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt; &amp; more"));
    }

    #[test]
    fn a_shadowed_legacy_store_is_reported() {
        let directory = tempfile::tempdir().unwrap();
        let legacy = directory.path().join(".pi").join("vector-memory");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("records.jsonl"), "{}\n{}\n").unwrap();
        let active = directory.path().join(".davinci").join("vector-memory");
        std::fs::create_dir_all(&active).unwrap();
        std::fs::write(active.join("records.jsonl"), "").unwrap();
        let config = VectorMemoryConfig {
            enabled: false,
            ..VectorMemoryConfig::default()
        };
        let memory = VectorMemory::with_config(directory.path().to_path_buf(), config);
        let report = build_report(
            &memory,
            &PageOptions {
                query: None,
                open: false,
            },
        );
        assert_eq!(report.verdict, Verdict::Disabled);
        assert_eq!(
            report.legacy_records.as_ref().map(|(_, count)| *count),
            Some(2)
        );
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.contains("legacy store")));
    }
}
