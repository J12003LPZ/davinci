//! Core analysis engine for P9 Change Impact Engine.
//! Composes AST, LSP, Package, TestMap, Build, Git, Config and Transaction evidence
//! into an honest, bounded, structured blast radius report.

use super::model::*;
use crate::{
    native_extensions::{
        build_intelligence::BuildIntelligence,
        git_intelligence::GitIntelligence,
        language_intelligence::LanguageIntelligence,
        package_intelligence::PackageIntelligence,
        repo_intelligence::{parse_source, RepoIntelligence, SourceRange},
        test_impact::TestImpact,
    },
    semantic::SemanticClient,
};
use davinci_agent::semantic::SemanticRequestContext;
use serde_json::{json, Value};
use std::{collections::BTreeSet, fs, path::Path};

pub struct ChangeImpactAnalyzer<'a> {
    root: &'a Path,
    config: &'a ChangeImpactConfig,
    repo: &'a RepoIntelligence,
    test_impact: &'a TestImpact,
    _package_intelligence: &'a PackageIntelligence,
    build_intelligence: &'a BuildIntelligence,
    git_intelligence: &'a GitIntelligence,
    language: &'a LanguageIntelligence,
}

impl<'a> ChangeImpactAnalyzer<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        root: &'a Path,
        config: &'a ChangeImpactConfig,
        repo: &'a RepoIntelligence,
        test_impact: &'a TestImpact,
        package_intelligence: &'a PackageIntelligence,
        build_intelligence: &'a BuildIntelligence,
        git_intelligence: &'a GitIntelligence,
        language: &'a LanguageIntelligence,
    ) -> Self {
        Self {
            root,
            config,
            repo,
            test_impact,
            _package_intelligence: package_intelligence,
            build_intelligence,
            git_intelligence,
            language,
        }
    }

    pub fn analyze(
        &self,
        files: Option<Vec<String>>,
        symbols: Option<Vec<String>>,
        transaction_id: Option<String>,
        scope: Option<String>,
        limit: Option<usize>,
    ) -> Result<ChangeImpactReport, String> {
        let scope = match scope.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(raw) => {
                let normalized = self.normalize_path(raw)?;
                let normalized = normalized.trim_matches('/').to_string();
                (!normalized.is_empty() && normalized != ".").then_some(normalized)
            }
            None => None,
        };
        let mut target_files = BTreeSet::new();
        let mut target_symbols = BTreeSet::new();
        let mut warnings = Vec::new();
        let mut is_partial = false;

        // 1. Resolve transaction ID if provided
        if let Some(ref tx_id) = transaction_id {
            self.resolve_transaction(tx_id, &mut target_files, &mut target_symbols, &mut warnings)?;
        }

        // 2. Resolve explicitly provided files
        if let Some(explicit_files) = files {
            for f in explicit_files {
                let norm = self.normalize_path(&f)?;
                target_files.insert(norm);
            }
        }

        // 3. Resolve explicitly provided symbols
        if let Some(explicit_symbols) = symbols {
            for s in explicit_symbols {
                target_symbols.insert(s.trim().to_string());
            }
        }

        // 4. If neither files nor symbols are provided, inspect the working tree
        // first (staged, unstaged and untracked edits), then committed history.
        if target_files.is_empty() && target_symbols.is_empty() {
            let working_tree = self.working_tree_changes();
            if !working_tree.is_empty() {
                warnings.push(
                    "Changed files taken from the working tree (staged, unstaged and untracked)"
                        .to_string(),
                );
                target_files.extend(working_tree);
            }
        }
        if target_files.is_empty() && target_symbols.is_empty() {
            warnings.push(
                "Working tree is clean; changed files taken from the latest commit comparison"
                    .to_string(),
            );
            if let Ok(res) = self
                .git_intelligence
                .execute_tool("git_changed_symbols", &json!({}))
            {
                if let Some(details) = res.details {
                    if let Some(symbols) = details.get("symbols").and_then(|s| s.as_array()) {
                        for s in symbols {
                            if let Some(name) = s.get("name").and_then(|n| n.as_str()) {
                                target_symbols.insert(name.to_string());
                            }
                            if let Some(file) = s.get("file").and_then(|f| f.as_str()) {
                                if let Ok(norm) = self.normalize_path(file) {
                                    target_files.insert(norm);
                                }
                            }
                        }
                    }
                }
            }
            if target_files.is_empty() {
                if let Ok(res) = self
                    .git_intelligence
                    .execute_tool("git_branch_diff", &json!({}))
                {
                    if let Some(details) = res.details {
                        if let Some(files) = details.get("files").and_then(|f| f.as_array()) {
                            for f in files {
                                if let Some(path) = f.get("path").and_then(|p| p.as_str()) {
                                    if let Ok(norm) = self.normalize_path(path) {
                                        target_files.insert(norm);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if let Some(scope) = &scope {
            let prefix = format!("{scope}/");
            let before = target_files.len();
            target_files.retain(|file| file == scope || file.starts_with(&prefix));
            if target_files.len() < before {
                warnings.push(format!(
                    "{} changed file(s) outside scope '{scope}' were excluded",
                    before - target_files.len()
                ));
                is_partial = true;
            }
        }

        if target_files.is_empty() && target_symbols.is_empty() {
            return Err(
                "No changed files, symbols, or transaction ID specified for impact analysis"
                    .to_string(),
            );
        }

        // Check file limit
        if target_files.len() > self.config.max_files {
            warnings.push(format!(
                "Target file count ({}) exceeded configured maxFiles ({}); results truncated",
                target_files.len(),
                self.config.max_files
            ));
            is_partial = true;
        }

        let file_list: Vec<String> = target_files
            .iter()
            .take(self.config.max_files)
            .cloned()
            .collect();
        let symbol_list: Vec<String> = target_symbols.iter().cloned().collect();
        let scope_prefix = scope.as_ref().map(|scope| format!("{scope}/"));

        // 5. Direct Semantic Impact (LSP)
        let semantic_limit = limit
            .unwrap_or(self.config.max_references)
            .clamp(1, self.config.max_references);
        let (semantic_impact, lsp_partial) =
            self.analyze_semantic(&file_list, &symbol_list, semantic_limit, &mut warnings);
        if lsp_partial {
            is_partial = true;
        }

        // 6. Structural Impact (AST)
        let structural_impact =
            self.analyze_structural(&file_list, &symbol_list, scope_prefix.as_deref());

        // 7. Tests (TestImpact)
        let tests_impact = self.analyze_tests(
            &file_list,
            &symbol_list,
            scope_prefix.as_deref(),
            &mut warnings,
            &mut is_partial,
        );

        // 8. Packages (PackageIntelligence / WorkspaceMetadata)
        let packages_impact = self.analyze_packages(&file_list);

        // 9. Build Targets (BuildIntelligence)
        let build_targets_impact = self.analyze_build_targets(&file_list);

        // 10. Public API Risk
        let public_api_risk = self.analyze_public_api(&file_list, &symbol_list);

        // 11. Configuration Impact
        let config_impact = self.analyze_configuration(&file_list);

        // 12. Potential Browser Flows
        let browser_flows = self.analyze_browser_flows(&file_list, &symbol_list);

        // 13. Completeness & Confidence
        let completeness = if is_partial {
            AnalysisCompleteness::Partial
        } else {
            AnalysisCompleteness::Complete
        };

        let confidence = if completeness == AnalysisCompleteness::Complete
            && semantic_impact.lsp_status == "available"
        {
            "HIGH".to_string()
        } else if !structural_impact.items.is_empty() || !tests_impact.items.is_empty() {
            "MEDIUM".to_string()
        } else {
            "LOW".to_string()
        };

        Ok(ChangeImpactReport {
            files: file_list,
            symbols: symbol_list,
            transaction_id,
            completeness,
            confidence,
            warnings,
            direct_semantic_impact: semantic_impact,
            structural_impact,
            tests: tests_impact,
            packages: packages_impact,
            build_targets: build_targets_impact,
            public_api_risk,
            configuration_impact: config_impact,
            potential_browser_flows: browser_flows,
        })
    }

    fn normalize_path(&self, raw: &str) -> Result<String, String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err("empty path not allowed".into());
        }
        if trimmed.contains('\0') {
            return Err("null bytes not allowed in path".into());
        }
        let normalized = trimmed.replace('\\', "/");
        if normalized.starts_with('/') || normalized.contains("../") || normalized == ".." {
            return Err(format!("path traversal rejected: {raw}"));
        }
        Ok(normalized)
    }

    fn resolve_transaction(
        &self,
        tx_id: &str,
        target_files: &mut BTreeSet<String>,
        target_symbols: &mut BTreeSet<String>,
        warnings: &mut Vec<String>,
    ) -> Result<(), String> {
        if tx_id.contains('/') || tx_id.contains('\\') || tx_id.contains("..") {
            return Err(format!("invalid transaction ID format: {tx_id}"));
        }
        // Records live in the Git directory for Git work trees; older ones may
        // still sit in the in-tree store.
        let file = format!("{tx_id}.json");
        let root = self
            .root
            .canonicalize()
            .unwrap_or_else(|_| self.root.to_path_buf());
        let record_path = [
            davinci_agent::runtime::transactions::transaction_store_dir(&root),
            root.join(".davinci-transactions"),
        ]
        .into_iter()
        .map(|dir| dir.join(&file))
        .find(|path| path.exists())
        .unwrap_or_else(|| root.join(".davinci-transactions").join(&file));
        if !record_path.exists() {
            warnings.push(format!("Transaction ID {tx_id} record not found on disk"));
            return Ok(());
        }

        let content = fs::read_to_string(&record_path)
            .map_err(|e| format!("failed to read transaction record {tx_id}: {e}"))?;
        let parsed: Value = serde_json::from_str(&content)
            .map_err(|e| format!("corrupt transaction record {tx_id}: {e}"))?;

        if let Some(files) = parsed["summary"]["affected_files"].as_array() {
            for f in files {
                if let Some(s) = f.as_str() {
                    if let Ok(norm) = self.normalize_path(s) {
                        target_files.insert(norm);
                    }
                }
            }
        }

        // Also check changes for symbol changes
        if let Some(changes) = parsed["changes"].as_array() {
            for change in changes {
                if let Some(path) = change["path"].as_str() {
                    if let Ok(norm) = self.normalize_path(path) {
                        target_files.insert(norm.clone());
                        // If proposed_bytes or proposed content is available, parse AST symbols
                        if let Some(proposed_bytes) = change["proposed_bytes"].as_array() {
                            let bytes: Vec<u8> = proposed_bytes
                                .iter()
                                .filter_map(|b| b.as_u64().map(|v| v as u8))
                                .collect();
                            if let Ok(text) = String::from_utf8(bytes) {
                                if let Ok(parsed) = parse_source(&norm, &text) {
                                    for sym in parsed.symbols {
                                        target_symbols.insert(sym.name);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn analyze_semantic(
        &self,
        files: &[String],
        symbols: &[String],
        limit: usize,
        warnings: &mut Vec<String>,
    ) -> (DirectSemanticImpact, bool) {
        let client = if std::env::var_os("PI_GRAPH_ROLE").is_some() {
            match davinci_agent::runtime::task_transport::TaskCoordinatorClient::from_env() {
                Some(parent) => SemanticClient::parent(parent),
                None => {
                    warnings.push(
                        "Parent semantic transport is unavailable; worker-local language servers are forbidden"
                            .into(),
                    );
                    return (
                        DirectSemanticImpact {
                            items: Vec::new(),
                            lsp_status: "unavailable".into(),
                            reference_count: 0,
                            summary: "Semantic analysis unavailable; no worker-local fallback was started".into(),
                        },
                        true,
                    );
                }
            }
        } else {
            let status = self.language.status();
            if !status["enabled"].as_bool().unwrap_or(false) {
                warnings.push("Language intelligence is disabled in settings; semantic LSP references are unavailable".into());
                return (
                    DirectSemanticImpact {
                        items: Vec::new(),
                        lsp_status: "disabled".into(),
                        reference_count: 0,
                        summary: "Semantic LSP analysis is disabled; blast radius remains partial"
                            .into(),
                    },
                    true,
                );
            }
            SemanticClient::local(self.language.clone())
        };

        let context = SemanticRequestContext::default();
        let mut items = Vec::new();
        let mut reference_count = 0usize;
        let mut lsp_failed = false;
        let mut remaining = limit;

        // Workspace symbols are declaration candidates only. Anchor them to a
        // real source path, then ask references for actual usage evidence.
        for symbol in symbols {
            if remaining == 0 {
                break;
            }
            let Some(anchor_file) = files.first() else {
                lsp_failed = true;
                warnings.push(format!(
                    "Cannot semantically anchor symbol '{symbol}' without a changed source file"
                ));
                continue;
            };
            let candidates = client.execute_tool(
                "lsp_workspace_symbols",
                &json!({"query":symbol,"path":anchor_file,"limit":remaining.min(20)}),
                &context,
            );
            match candidates {
                Ok(output) if !output.is_error => {
                    let Some(details) = output.details else {
                        lsp_failed = true;
                        continue;
                    };
                    let Some(candidate) = details["items"].as_array().and_then(|items| {
                        items
                            .iter()
                            .find(|item| item["name"].as_str() == Some(symbol.as_str()))
                            .or_else(|| items.first())
                    }) else {
                        continue;
                    };
                    let path = candidate["path"].as_str().unwrap_or(anchor_file);
                    let Some(range) = normalized_source_range(candidate.get("range")) else {
                        lsp_failed = true;
                        continue;
                    };
                    match client.execute_tool(
                        "lsp_references",
                        &json!({
                            "path":path,
                            "line":range.start_line,
                            "column":range.start_column,
                            "includeDeclaration":true,
                            "limit":remaining
                        }),
                        &context,
                    ) {
                        Ok(output) => {
                            if record_semantic_references(
                                symbol,
                                output,
                                &mut remaining,
                                &mut reference_count,
                                &mut items,
                            )
                            .is_err()
                            {
                                lsp_failed = true;
                            }
                        }
                        Err(_) => lsp_failed = true,
                    }
                }
                _ => lsp_failed = true,
            }
        }

        // For changed files, document symbols become source anchors; each one
        // is resolved through references until the aggregate semantic budget is spent.
        for file in files {
            if remaining == 0 {
                break;
            }
            let symbols_output = client.execute_tool(
                "lsp_document_symbols",
                &json!({"path":file,"limit":remaining.min(20)}),
                &context,
            );
            let Ok(symbols_output) = symbols_output else {
                lsp_failed = true;
                continue;
            };
            if symbols_output.is_error {
                lsp_failed = true;
                continue;
            }
            let Some(details) = symbols_output.details else {
                lsp_failed = true;
                continue;
            };
            for symbol in details["items"].as_array().into_iter().flatten() {
                if remaining == 0 {
                    break;
                }
                let name = symbol["name"].as_str().unwrap_or("<symbol>");
                let Some(range) = normalized_source_range(symbol.get("range")) else {
                    lsp_failed = true;
                    continue;
                };
                match client.execute_tool(
                    "lsp_references",
                    &json!({
                        "path":file,
                        "line":range.start_line,
                        "column":range.start_column,
                        "includeDeclaration":true,
                        "limit":remaining
                    }),
                    &context,
                ) {
                    Ok(output) => {
                        if record_semantic_references(
                            name,
                            output,
                            &mut remaining,
                            &mut reference_count,
                            &mut items,
                        )
                        .is_err()
                        {
                            lsp_failed = true;
                        }
                    }
                    Err(_) => lsp_failed = true,
                }
            }
        }

        if remaining == 0 {
            warnings.push(format!(
                "Semantic reference analysis reached its aggregate limit of {limit}; coverage is partial"
            ));
            lsp_failed = true;
        }

        let lsp_status = if lsp_failed && items.is_empty() {
            warnings.push("Language intelligence is unavailable or incomplete; semantic impact remains uncertain".into());
            "unavailable".to_string()
        } else if lsp_failed {
            warnings.push(
                "Some semantic queries failed or were truncated; impact evidence is partial".into(),
            );
            "partial".to_string()
        } else {
            "available".to_string()
        };
        let summary = format!(
            "{reference_count} actual semantic reference locations discovered via LSP ({lsp_status})"
        );
        (
            DirectSemanticImpact {
                items,
                lsp_status,
                reference_count,
                summary,
            },
            lsp_failed,
        )
    }

    fn analyze_structural(
        &self,
        files: &[String],
        symbols: &[String],
        scope_prefix: Option<&str>,
    ) -> StructuralImpact {
        let mut items = Vec::new();
        let mut importing_modules = BTreeSet::new();
        let mut imported_modules = BTreeSet::new();

        let index_res = self.repo.refresh();
        if let Ok(index) = index_res {
            for (file_path, repo_file) in &index.files {
                if scope_prefix.is_some_and(|prefix| !file_path.starts_with(prefix)) {
                    continue;
                }
                // Refresh structural evidence from the live source when
                // possible. The repository index may legitimately retain a
                // file entry while its parsed import list is stale during a
                // just-created test/worktree snapshot.
                let live_parse = fs::read_to_string(self.root.join(file_path))
                    .ok()
                    .and_then(|source| parse_source(file_path, &source).ok());
                let imports = live_parse
                    .as_ref()
                    .map(|parsed| parsed.imports.as_slice())
                    .unwrap_or(repo_file.imports.as_slice());
                for import in imports {
                    let source = &import.specifier;
                    for target in files {
                        if import_matches_target(file_path, source, target) && file_path != target {
                            importing_modules.insert(file_path.clone());
                            items.push(ImpactItem {
                                name: file_path.clone(),
                                path: file_path.clone(),
                                evidence_source: EvidenceSource::Ast,
                                description: format!(
                                    "Module '{file_path}' structurally imports '{target}'"
                                ),
                                range: Some(SourceRange {
                                    start_line: import.line,
                                    start_column: 0,
                                    end_line: import.line,
                                    end_column: 0,
                                }),
                                details: None,
                            });
                        }
                    }
                }

                // Check symbols imported/exported
                for sym in symbols {
                    if repo_file.symbols.iter().any(|s| &s.name == sym) {
                        items.push(ImpactItem {
                            name: sym.clone(),
                            path: file_path.clone(),
                            evidence_source: EvidenceSource::Ast,
                            description: format!("Symbol '{sym}' defined in '{file_path}'"),
                            range: None,
                            details: None,
                        });
                    }
                }
            }

            for target in files {
                if let Some(target_file) = index.files.get(target) {
                    for import in &target_file.imports {
                        imported_modules.insert(import.specifier.clone());
                    }
                }
            }
        }

        let summary = format!(
            "{} importing modules and {} imported dependencies identified via AST analysis",
            importing_modules.len(),
            imported_modules.len()
        );

        StructuralImpact {
            items,
            importing_modules: importing_modules.into_iter().collect(),
            imported_modules: imported_modules.into_iter().collect(),
            summary,
        }
    }

    /// Files with staged, unstaged or untracked changes, relative to the root.
    fn working_tree_changes(&self) -> Vec<String> {
        use crate::native_extensions::git_intelligence::runner;
        let mut files = BTreeSet::new();
        for args in [
            &["diff", "-z", "--name-only", "HEAD"][..],
            &["ls-files", "-z", "--others", "--exclude-standard"][..],
        ] {
            let Ok(out) = runner::run(self.root, args) else {
                continue;
            };
            for raw in out.split(|byte| *byte == 0) {
                let path = String::from_utf8_lossy(raw);
                if path.is_empty() {
                    continue;
                }
                if let Ok(normalized) = self.normalize_path(&path) {
                    files.insert(normalized);
                }
            }
        }
        files.into_iter().collect()
    }

    /// Resolves symbol names (or ids) to the AST symbol ids TestImpact needs.
    fn resolve_symbol_ids(
        &self,
        symbols: &[String],
        scope_prefix: Option<&str>,
        warnings: &mut Vec<String>,
    ) -> Vec<String> {
        if symbols.is_empty() {
            return Vec::new();
        }
        let Ok(index) = self.repo.refresh() else {
            warnings.push("Symbol ids could not be resolved: repository index unavailable".into());
            return Vec::new();
        };
        let mut ids = BTreeSet::new();
        for wanted in symbols {
            let matches: Vec<&str> = index
                .files
                .iter()
                .filter(|(path, _)| scope_prefix.is_none_or(|prefix| path.starts_with(prefix)))
                .flat_map(|(_, file)| &file.symbols)
                .filter(|symbol| symbol.id == *wanted || symbol.name == *wanted)
                .map(|symbol| symbol.id.as_str())
                .collect();
            match matches.len() {
                0 => warnings.push(format!(
                    "Symbol '{wanted}' not found in the index; no test evidence for it"
                )),
                1 => {
                    ids.insert(matches[0].to_string());
                }
                count => {
                    warnings.push(format!(
                        "Symbol '{wanted}' is ambiguous ({count} definitions); tests selected for all of them"
                    ));
                    ids.extend(matches.into_iter().map(str::to_string));
                }
            }
        }
        ids.into_iter().collect()
    }

    fn analyze_tests(
        &self,
        files: &[String],
        symbols: &[String],
        scope_prefix: Option<&str>,
        warnings: &mut Vec<String>,
        is_partial: &mut bool,
    ) -> TestsImpact {
        let mut items = Vec::new();
        let mut selected_tests = BTreeSet::new();
        let mut test_command = None;
        let mut broader_verification = false;

        let symbol_ids = self.resolve_symbol_ids(symbols, scope_prefix, warnings);
        let args = json!({
            "paths": files,
            "symbolIds": symbol_ids,
            "limit": 50
        });

        if files.is_empty() && symbol_ids.is_empty() {
            // TestImpact requires at least one resolvable input.
        } else {
            match self.test_impact.execute("test_impacted", &args) {
                Ok(res) => {
                    if let Some(details) = res.details {
                        if let Some(tests) = details.get("results").and_then(|t| t.as_array()) {
                            for t in tests {
                                if let Some(path) = t["path"].as_str() {
                                    selected_tests.insert(path.to_string());
                                    items.push(ImpactItem {
                                        name: path.to_string(),
                                        path: path.to_string(),
                                        evidence_source: EvidenceSource::TestMap,
                                        description: format!("Test impacted by change: {path}"),
                                        range: None,
                                        details: Some(t.clone()),
                                    });
                                }
                            }
                        }
                        if details.get("partial").and_then(|p| p.as_bool()) == Some(true) {
                            *is_partial = true;
                        }
                        if let Some(extra) = details.get("warnings").and_then(|w| w.as_array()) {
                            warnings.extend(
                                extra
                                    .iter()
                                    .filter_map(|w| w.as_str())
                                    .map(|w| format!("test impact: {w}")),
                            );
                        }
                        let commands = |key: &str| -> Vec<String> {
                            details
                                .get(key)
                                .and_then(|c| c.as_array())
                                .map(|list| list.iter().filter_map(render_command).collect())
                                .unwrap_or_default()
                        };
                        let first = commands("first_tier");
                        if !first.is_empty() {
                            test_command = Some(first.join(" && "));
                        }
                        broader_verification = !commands("broader_verification").is_empty();
                    }
                }
                Err(err) => {
                    warnings.push(format!("Test impact unavailable: {err}"));
                    *is_partial = true;
                }
            }
        }

        let summary = format!(
            "{} tests selected for execution based on AST test impact mapping",
            selected_tests.len()
        );

        TestsImpact {
            items,
            selected_tests: selected_tests.into_iter().collect(),
            test_command,
            broader_verification_required: broader_verification,
            summary,
        }
    }

    fn analyze_packages(&self, files: &[String]) -> PackagesImpact {
        let mut items = Vec::new();
        let mut affected_packages = BTreeSet::new();

        if let Ok(snapshot) = self.test_impact.workspace_facts(files, false) {
            let meta = &snapshot.metadata;
            for pkg in &meta.packages {
                let pkg_name = pkg.name.clone().unwrap_or_else(|| pkg.path.clone());
                if let Some(file) = files
                    .iter()
                    .find(|file| pkg.path == "." || file.starts_with(&format!("{}/", pkg.path)))
                {
                    affected_packages.insert(pkg_name.clone());
                    items.push(ImpactItem {
                        name: pkg_name.clone(),
                        path: pkg.path.clone(),
                        evidence_source: EvidenceSource::Package,
                        description: format!(
                            "Package '{pkg_name}' contains modified file '{file}'"
                        ),
                        range: None,
                        details: Some(
                            json!({"manifest":pkg.manifest,"source_identity":snapshot.identity}),
                        ),
                    });
                }
            }
            // Fixed point is bounded by the number of packages, including cycles.
            loop {
                let mut added = Vec::new();
                for pkg in &meta.packages {
                    let name = pkg.name.clone().unwrap_or_else(|| pkg.path.clone());
                    if affected_packages.contains(&name) {
                        continue;
                    }
                    if let Some(dependency) = pkg
                        .dependencies
                        .iter()
                        .find(|dep| affected_packages.contains(*dep))
                    {
                        added.push((name, pkg.path.clone(), dependency.clone()));
                    }
                }
                if added.is_empty() {
                    break;
                }
                for (name, path, dependency) in added {
                    affected_packages.insert(name.clone());
                    items.push(ImpactItem { name: name.clone(), path,
                        evidence_source: EvidenceSource::Package,
                        description: format!("Downstream package '{name}' depends on affected package '{dependency}'"),
                        range: None, details: Some(json!({"source_identity":snapshot.identity})),
                    });
                }
            }
        }

        let summary = format!(
            "{} workspace packages affected directly or transitively",
            affected_packages.len()
        );

        PackagesImpact {
            items,
            affected_packages: affected_packages.into_iter().collect(),
            summary,
        }
    }

    fn analyze_build_targets(&self, files: &[String]) -> BuildTargetsImpact {
        let mut items = Vec::new();
        let mut affected_targets = BTreeSet::new();

        let args = json!({ "files": files });
        if let Ok(res) = self
            .build_intelligence
            .execute_tool("build_affected", &args)
        {
            if let Some(details) = res.details {
                if let Some(targets) = details.get("affectedTargets").and_then(|t| t.as_array()) {
                    for t in targets {
                        if let Some(target_str) = t.as_str() {
                            affected_targets.insert(target_str.to_string());
                            items.push(ImpactItem {
                                name: target_str.to_string(),
                                path: target_str.to_string(),
                                evidence_source: EvidenceSource::Build,
                                description: format!("Build target affected: {target_str}"),
                                range: None,
                                details: None,
                            });
                        }
                    }
                }
            }
        }

        let summary = format!(
            "{} build targets impacted across workspace configuration",
            affected_targets.len()
        );

        BuildTargetsImpact {
            items,
            affected_targets: affected_targets.into_iter().collect(),
            summary,
        }
    }

    fn analyze_public_api(&self, files: &[String], symbols: &[String]) -> PublicApiRisk {
        let mut items = Vec::new();
        let mut reexports = Vec::new();
        let mut is_public = false;

        let entry_points = [
            "index.ts",
            "index.js",
            "src/index.ts",
            "src/index.js",
            "main.ts",
            "main.js",
            "lib.rs",
            "src/lib.rs",
        ];

        for f in files {
            for entry in entry_points {
                if f == entry || f.ends_with(&format!("/{entry}")) {
                    is_public = true;
                    items.push(ImpactItem {
                        name: f.clone(),
                        path: f.clone(),
                        evidence_source: EvidenceSource::Ast,
                        description: format!("Package entry point '{f}' modified"),
                        range: None,
                        details: None,
                    });
                }
            }
        }

        // Check if any symbols are exported in entry points or index files
        if let Ok(index) = self.repo.refresh() {
            for entry in entry_points {
                if let Some(file) = index.files.get(entry) {
                    for sym in symbols {
                        if file.symbols.iter().any(|s| &s.name == sym) {
                            is_public = true;
                            reexports.push(format!("{entry}:{sym}"));
                            items.push(ImpactItem {
                                name: sym.clone(),
                                path: entry.to_string(),
                                evidence_source: EvidenceSource::Ast,
                                description: format!("Exported public symbol '{sym}' in '{entry}'"),
                                range: None,
                                details: None,
                            });
                        }
                    }
                }
            }
        }

        let summary = if is_public {
            format!(
                "HIGH public API risk: {} public exports/entry points touched",
                items.len()
            )
        } else {
            "No public API entry points or root exports affected".to_string()
        };

        PublicApiRisk {
            items,
            is_public_api_affected: is_public,
            reexports,
            summary,
        }
    }

    fn analyze_configuration(&self, files: &[String]) -> ConfigurationImpact {
        let mut items = Vec::new();
        let mut affected_configs = BTreeSet::new();

        let config_patterns = [
            "package.json",
            "tsconfig.json",
            "tsconfig.",
            "vite.config.",
            "next.config.",
            "turbo.json",
            "nx.json",
            ".eslintrc",
            "Cargo.toml",
            "pnpm-workspace.yaml",
            ".env",
        ];

        for f in files {
            let filename = Path::new(f)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            for pat in config_patterns {
                if filename == pat || filename.starts_with(pat) {
                    affected_configs.insert(f.clone());
                    items.push(ImpactItem {
                        name: filename.clone(),
                        path: f.clone(),
                        evidence_source: EvidenceSource::Config,
                        description: format!("Project configuration file modified: {f}"),
                        range: None,
                        details: None,
                    });
                    break;
                }
            }
        }

        let is_config = !affected_configs.is_empty();
        let summary = if is_config {
            format!(
                "{} configuration files modified with potential workspace blast radius",
                affected_configs.len()
            )
        } else {
            "No configuration files modified".to_string()
        };

        ConfigurationImpact {
            items,
            is_config_affected: is_config,
            affected_configs: affected_configs.into_iter().collect(),
            summary,
        }
    }

    fn analyze_browser_flows(&self, files: &[String], symbols: &[String]) -> PotentialBrowserFlows {
        let mut items = Vec::new();
        let mut affected_flows = BTreeSet::new();
        let mut has_ui = false;

        let ui_extensions = [".tsx", ".jsx", ".html", ".vue", ".svelte", ".css", ".scss"];
        let ui_paths = [
            "pages/",
            "app/",
            "components/",
            "views/",
            "routes/",
            "src/ui/",
        ];

        for f in files {
            let is_ui_file = ui_extensions.iter().any(|ext| f.ends_with(ext))
                || ui_paths.iter().any(|p| f.contains(p));

            if is_ui_file {
                has_ui = true;
                let flow_name = if f.contains("login") || f.contains("auth") {
                    "Authentication Flow"
                } else if f.contains("checkout") || f.contains("cart") {
                    "Checkout Flow"
                } else if f.contains("dashboard") {
                    "Dashboard Flow"
                } else if f.contains("settings") {
                    "Settings Flow"
                } else {
                    "UI Component Flow"
                };

                affected_flows.insert(flow_name.to_string());
                items.push(ImpactItem {
                    name: flow_name.to_string(),
                    path: f.clone(),
                    evidence_source: EvidenceSource::Ast,
                    description: format!("UI file '{f}' mapped to '{flow_name}'"),
                    range: None,
                    details: None,
                });
            }
        }

        for sym in symbols {
            let lower = sym.to_lowercase();
            if lower.contains("login") || lower.contains("auth") {
                has_ui = true;
                affected_flows.insert("Authentication Flow".to_string());
            } else if lower.contains("button") || lower.contains("modal") || lower.contains("form")
            {
                has_ui = true;
                affected_flows.insert("Interactive Component Flow".to_string());
            }
        }

        let summary = if has_ui {
            format!(
                "{} potential browser flows identified across UI components",
                affected_flows.len()
            )
        } else {
            "No frontend UI files or browser flows affected".to_string()
        };

        PotentialBrowserFlows {
            items,
            affected_flows: affected_flows.into_iter().collect(),
            has_ui_impact: has_ui,
            summary,
        }
    }
}

fn record_semantic_references(
    name: &str,
    output: davinci_agent::ToolResult,
    remaining: &mut usize,
    reference_count: &mut usize,
    items: &mut Vec<ImpactItem>,
) -> Result<(), ()> {
    if output.is_error {
        return Err(());
    }
    let details = output.details.ok_or(())?;
    for item in details["items"].as_array().into_iter().flatten() {
        if *remaining == 0 {
            break;
        }
        let path = item["path"].as_str().ok_or(())?;
        let range = normalized_source_range(item.get("range")).ok_or(())?;
        *remaining -= 1;
        *reference_count += 1;
        items.push(ImpactItem {
            name: name.to_string(),
            path: path.to_string(),
            evidence_source: EvidenceSource::Lsp,
            description: format!(
                "Semantic reference to '{name}' at {path}:{}:{}",
                range.start_line, range.start_column
            ),
            range: Some(range),
            details: Some(item.clone()),
        });
    }
    Ok(())
}

fn normalized_source_range(value: Option<&Value>) -> Option<SourceRange> {
    let value = value?;
    let start_line = value["start"]["line"].as_u64()? as usize;
    let start_column = value["start"]["column"].as_u64()? as usize;
    let end_line = value["end"]["line"].as_u64()? as usize;
    let end_column = value["end"]["column"].as_u64()? as usize;
    (start_line > 0 && start_column > 0 && end_line > 0 && end_column > 0).then_some(SourceRange {
        start_line,
        start_column,
        end_line,
        end_column,
    })
}

/// Collapses `.` and `..` segments of a slash-separated relative path.
fn collapse_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

/// Whether `specifier`, imported from `importing_file`, names `target_file`.
/// Relative specifiers resolve against the importing directory; others match
/// only whole path segments. Names that merely end alike never match.
fn import_matches_target(importing_file: &str, specifier: &str, target_file: &str) -> bool {
    let target_stem = [".ts", ".tsx", ".js", ".jsx", ".mts", ".cts", ".mjs", ".cjs"]
        .iter()
        .find_map(|ext| target_file.strip_suffix(ext))
        .unwrap_or(target_file);
    let target_index_dir = target_stem.strip_suffix("/index");
    let names_target = |candidate: &str| {
        candidate == target_file
            || candidate == target_stem
            || target_index_dir.is_some_and(|dir| candidate == dir)
    };

    if specifier.starts_with('.') {
        let parent = Path::new(importing_file)
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or("")
            .replace('\\', "/");
        let resolved = collapse_path(&format!("{parent}/{specifier}"));
        return names_target(&resolved);
    }

    let spec = collapse_path(specifier);
    names_target(&spec)
        || (spec.contains('/')
            && [target_file, target_stem]
                .iter()
                .any(|name| name.ends_with(&format!("/{spec}"))))
}

/// `program arg arg ...` for a TestImpact verification command object.
fn render_command(command: &Value) -> Option<String> {
    let program = command.get("program")?.as_str()?;
    let args = command
        .get("argv")
        .and_then(|a| a.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();
    Some(
        std::iter::once(program)
            .chain(args)
            .collect::<Vec<_>>()
            .join(" "),
    )
}
