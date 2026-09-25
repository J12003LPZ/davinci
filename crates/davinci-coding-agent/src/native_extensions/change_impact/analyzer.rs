//! Core analysis engine for P9 Change Impact Engine.
//! Composes AST, LSP, Package, TestMap, Build, Git, Config and Transaction evidence
//! into an honest, bounded, structured blast radius report.

use super::model::*;
use crate::native_extensions::{
    build_intelligence::BuildIntelligence,
    git_intelligence::GitIntelligence,
    language_intelligence::LanguageIntelligence,
    package_intelligence::PackageIntelligence,
    repo_intelligence::{parse_source, RepoIntelligence, SourceRange},
    test_impact::TestImpact,
};
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
        _scope: Option<String>,
        _limit: Option<usize>,
    ) -> Result<ChangeImpactReport, String> {
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

        // 4. If neither files nor symbols are provided, inspect working tree via git or repo
        if target_files.is_empty() && target_symbols.is_empty() {
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

        // 5. Direct Semantic Impact (LSP)
        let (semantic_impact, lsp_partial) =
            self.analyze_semantic(&file_list, &symbol_list, &mut warnings);
        if lsp_partial {
            is_partial = true;
        }

        // 6. Structural Impact (AST)
        let structural_impact = self.analyze_structural(&file_list, &symbol_list);

        // 7. Tests (TestImpact)
        let tests_impact = self.analyze_tests(&file_list, &symbol_list);

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
        let store_dir = self.root.join(".davinci-transactions");
        let record_path = store_dir.join(format!("{tx_id}.json"));
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
        warnings: &mut Vec<String>,
    ) -> (DirectSemanticImpact, bool) {
        let status = self.language.status();
        let enabled = status["enabled"].as_bool().unwrap_or(false);
        if !enabled {
            warnings.push("Language intelligence is disabled in settings; semantic LSP references are unavailable".into());
            return (
                DirectSemanticImpact {
                    items: Vec::new(),
                    lsp_status: "disabled".into(),
                    reference_count: 0,
                    summary: "Semantic LSP analysis is disabled; blast radius is based on structural AST only".into(),
                },
                true,
            );
        }

        let mut items = Vec::new();
        let mut reference_count = 0;
        let mut lsp_failed = false;

        // Workspace symbols are declaration candidates, not usage evidence.
        // Resolve each candidate to actual references before incrementing counts.
        for sym in symbols {
            let res = self.language.execute(
                "lsp_workspace_symbols",
                &json!({ "query": sym, "limit": self.config.max_references }),
            );
            match res {
                Ok(out) if !out.is_error => {
                    if let Some(details) = out.details {
                        if let Some(arr) = details.get("items").and_then(Value::as_array) {
                            for declaration in arr.iter().take(self.config.max_references) {
                                let Some(path) = declaration.get("path").and_then(Value::as_str) else {
                                    lsp_failed = true;
                                    continue;
                                };
                                let Some(line) = declaration
                                    .pointer("/range/start/line")
                                    .and_then(Value::as_u64)
                                else {
                                    lsp_failed = true;
                                    continue;
                                };
                                let Some(column) = declaration
                                    .pointer("/range/start/column")
                                    .and_then(Value::as_u64)
                                else {
                                    lsp_failed = true;
                                    continue;
                                };
                                let refs = self.language.execute(
                                    "lsp_references",
                                    &json!({
                                        "path": path,
                                        "line": line,
                                        "column": column,
                                        "includeDeclaration": false,
                                        "limit": self.config.max_references
                                    }),
                                );
                                match refs {
                                    Ok(refs) if !refs.is_error => {
                                        if let Some(reference_items) = refs
                                            .details
                                            .as_ref()
                                            .and_then(|details| details.get("items"))
                                            .and_then(Value::as_array)
                                        {
                                            for item in reference_items
                                                .iter()
                                                .take(self.config.max_references.saturating_sub(reference_count))
                                            {
                                                let Some(item_path) =
                                                    item.get("path").and_then(Value::as_str)
                                                else {
                                                    lsp_failed = true;
                                                    continue;
                                                };
                                                let Some(start_line) = item
                                                    .pointer("/range/start/line")
                                                    .and_then(Value::as_u64)
                                                else {
                                                    lsp_failed = true;
                                                    continue;
                                                };
                                                let Some(start_col) = item
                                                    .pointer("/range/start/column")
                                                    .and_then(Value::as_u64)
                                                else {
                                                    lsp_failed = true;
                                                    continue;
                                                };
                                                let end_line = item
                                                    .pointer("/range/end/line")
                                                    .and_then(Value::as_u64)
                                                    .unwrap_or(start_line);
                                                let end_col = item
                                                    .pointer("/range/end/column")
                                                    .and_then(Value::as_u64)
                                                    .unwrap_or(start_col);
                                                reference_count += 1;
                                                items.push(ImpactItem {
                                                    name: sym.clone(),
                                                    path: item_path.to_string(),
                                                    evidence_source: EvidenceSource::Lsp,
                                                    description: format!(
                                                        "Semantic reference to symbol '{sym}' at {item_path}:{start_line}:{start_col}"
                                                    ),
                                                    range: Some(SourceRange {
                                                        start_line: start_line as usize,
                                                        start_column: start_col as usize,
                                                        end_line: end_line as usize,
                                                        end_column: end_col as usize,
                                                    }),
                                                    details: Some(item.clone()),
                                                });
                                                if reference_count >= self.config.max_references {
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                    _ => lsp_failed = true,
                                }
                                if reference_count >= self.config.max_references {
                                    break;
                                }
                            }
                        }
                    }
                }
                _ => lsp_failed = true,
            }
            if reference_count >= self.config.max_references {
                break;
            }
        }

        // For changed files, use document symbols only as anchors, then query
        // references at their normalized selection ranges.
        for file in files {
            let res = self
                .language
                .execute("lsp_document_symbols", &json!({ "path": file, "limit": self.config.max_references }));
            match res {
                Ok(out) if !out.is_error => {
                    if let Some(details) = out.details {
                        if let Some(arr) = details.get("items").and_then(Value::as_array) {
                            for declaration in arr
                                .iter()
                                .take(self.config.max_references.saturating_sub(reference_count))
                            {
                                let name = declaration
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .unwrap_or("")
                                    .to_string();
                                let Some(line) = declaration
                                    .pointer("/range/start/line")
                                    .and_then(Value::as_u64)
                                else {
                                    lsp_failed = true;
                                    continue;
                                };
                                let Some(column) = declaration
                                    .pointer("/range/start/column")
                                    .and_then(Value::as_u64)
                                else {
                                    lsp_failed = true;
                                    continue;
                                };
                                match self.language.execute(
                                    "lsp_references",
                                    &json!({
                                        "path": file,
                                        "line": line,
                                        "column": column,
                                        "includeDeclaration": false,
                                        "limit": self.config.max_references
                                    }),
                                ) {
                                    Ok(refs) if !refs.is_error => {
                                        if let Some(reference_items) = refs
                                            .details
                                            .as_ref()
                                            .and_then(|details| details.get("items"))
                                            .and_then(Value::as_array)
                                        {
                                            for item in reference_items
                                                .iter()
                                                .take(self.config.max_references.saturating_sub(reference_count))
                                            {
                                                let Some(item_path) =
                                                    item.get("path").and_then(Value::as_str)
                                                else {
                                                    lsp_failed = true;
                                                    continue;
                                                };
                                                let Some(start_line) = item
                                                    .pointer("/range/start/line")
                                                    .and_then(Value::as_u64)
                                                else {
                                                    lsp_failed = true;
                                                    continue;
                                                };
                                                let Some(start_col) = item
                                                    .pointer("/range/start/column")
                                                    .and_then(Value::as_u64)
                                                else {
                                                    lsp_failed = true;
                                                    continue;
                                                };
                                                let end_line = item
                                                    .pointer("/range/end/line")
                                                    .and_then(Value::as_u64)
                                                    .unwrap_or(start_line);
                                                let end_col = item
                                                    .pointer("/range/end/column")
                                                    .and_then(Value::as_u64)
                                                    .unwrap_or(start_col);
                                                reference_count += 1;
                                                items.push(ImpactItem {
                                                    name: name.clone(),
                                                    path: item_path.to_string(),
                                                    evidence_source: EvidenceSource::Lsp,
                                                    description: format!(
                                                        "Semantic reference to '{name}' at {item_path}:{start_line}:{start_col}"
                                                    ),
                                                    range: Some(SourceRange {
                                                        start_line: start_line as usize,
                                                        start_column: start_col as usize,
                                                        end_line: end_line as usize,
                                                        end_column: end_col as usize,
                                                    }),
                                                    details: Some(item.clone()),
                                                });
                                                if reference_count >= self.config.max_references {
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                    _ => lsp_failed = true,
                                }
                                if reference_count >= self.config.max_references {
                                    break;
                                }
                            }
                        }
                    }
                }
                _ => lsp_failed = true,
            }
            if reference_count >= self.config.max_references {
                break;
            }
        }

        let lsp_status = if lsp_failed && items.is_empty() {
            warnings.push("Language intelligence server is unavailable or unconfigured; semantic analysis is partial".into());
            "unavailable".to_string()
        } else if lsp_failed {
            warnings.push(
                "Some LSP queries failed during semantic analysis; results are partial".into(),
            );
            "partial".to_string()
        } else {
            "available".to_string()
        };

        let summary = format!(
            "{reference_count} direct semantic references discovered via LSP ({lsp_status})"
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

    fn analyze_structural(&self, files: &[String], symbols: &[String]) -> StructuralImpact {
        let mut items = Vec::new();
        let mut importing_modules = BTreeSet::new();
        let mut imported_modules = BTreeSet::new();

        let index_res = self.repo.refresh();
        if let Ok(index) = index_res {
            for (file_path, repo_file) in &index.files {
                for import in &repo_file.imports {
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

    fn analyze_tests(&self, files: &[String], symbols: &[String]) -> TestsImpact {
        let mut items = Vec::new();
        let mut selected_tests = BTreeSet::new();
        let mut test_command = None;
        let mut broader_verification = false;

        let args = json!({
            "paths": files,
            "symbolIds": symbols,
            "limit": 50
        });

        if let Ok(res) = self.test_impact.execute("test_impacted", &args) {
            if let Some(details) = res.details {
                if let Some(tests) = details.get("tests").and_then(|t| t.as_array()) {
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
            }
        }

        // Also check test_plan
        if let Ok(plan_res) = self.test_impact.execute("test_plan", &args) {
            if let Some(details) = plan_res.details {
                if let Some(cmd) = details.get("command").and_then(|c| c.as_str()) {
                    test_command = Some(cmd.to_string());
                }
                if let Some(broader) = details
                    .get("broaderVerificationRequired")
                    .and_then(|b| b.as_bool())
                {
                    broader_verification = broader;
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

fn import_matches_target(importing_file: &str, specifier: &str, target_file: &str) -> bool {
    let spec = specifier.trim_start_matches("./");
    let target_stem = target_file
        .strip_suffix(".ts")
        .or_else(|| target_file.strip_suffix(".tsx"))
        .or_else(|| target_file.strip_suffix(".js"))
        .or_else(|| target_file.strip_suffix(".jsx"))
        .unwrap_or(target_file);

    if spec == target_file || spec == target_stem {
        return true;
    }

    if let Some(parent) = Path::new(importing_file).parent() {
        let parent_str = parent.to_str().unwrap_or("").replace('\\', "/");
        let resolved = if parent_str.is_empty() || parent_str == "." {
            spec.to_string()
        } else {
            format!("{parent_str}/{spec}")
        };
        if resolved == target_file || resolved == target_stem {
            return true;
        }
    }

    target_file.ends_with(spec) || target_stem.ends_with(spec)
}
