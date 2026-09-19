use super::model::*;
use super::runners::*;
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::path::{Path, PathBuf};

pub struct BuildResolver {
    root: PathBuf,
}

impl BuildResolver {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    pub fn discover_packages(
        &self,
        scope: Option<&str>,
    ) -> Result<WorkspacePackagesResult, String> {
        self.validate_scope(scope)?;
        let pm = detect_package_manager(&self.root);
        let pm_str = pm.as_str().to_string();
        let mut packages = Vec::new();

        let root_pkg_json = self.root.join("package.json");
        let root_manifest = if root_pkg_json.is_file() {
            let content = std::fs::read_to_string(&root_pkg_json)
                .map_err(|e| format!("failed to read root package.json: {e}"))?;
            serde_json::from_str::<serde_json::Value>(&content).ok()
        } else {
            None
        };

        // Check monorepo workspace patterns
        let mut workspace_patterns = Vec::new();
        if let Some(ref manifest) = root_manifest {
            if let Some(ws) = manifest.get("workspaces") {
                if let Some(arr) = ws.as_array() {
                    for item in arr {
                        if let Some(s) = item.as_str() {
                            workspace_patterns.push(s.to_string());
                        }
                    }
                } else if let Some(obj) = ws.as_object() {
                    if let Some(packages_arr) = obj.get("packages").and_then(|p| p.as_array()) {
                        for item in packages_arr {
                            if let Some(s) = item.as_str() {
                                workspace_patterns.push(s.to_string());
                            }
                        }
                    }
                }
            }
        }

        // Also check pnpm-workspace.yaml
        let pnpm_ws_path = self.root.join("pnpm-workspace.yaml");
        if pnpm_ws_path.is_file() {
            if let Ok(content) = std::fs::read_to_string(&pnpm_ws_path) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if let Some(stripped) = trimmed.strip_prefix('-') {
                        let pat = stripped.trim().trim_matches('\'').trim_matches('"');
                        if !pat.is_empty() {
                            workspace_patterns.push(pat.to_string());
                        }
                    }
                }
            }
        }

        // Also discover packages from root tsconfig project references
        if let Some((_, root_ts)) = parse_tsconfig(&self.root) {
            for ref_entry in root_ts.references {
                let ref_dir = self.root.join(&ref_entry.path);
                if ref_dir.is_dir() && ref_dir.join("package.json").is_file() {
                    if let Ok(rel) = ref_dir.strip_prefix(&self.root) {
                        let pat = rel.to_string_lossy().replace('\\', "/");
                        if !workspace_patterns.contains(&pat) {
                            workspace_patterns.push(pat);
                        }
                    }
                }
            }
        }

        let is_monorepo = !workspace_patterns.is_empty();

        if is_monorepo {
            for pattern in &workspace_patterns {
                let matched_dirs = self.match_workspace_glob(pattern);
                for dir in matched_dirs {
                    if let Some(pkg) = self.load_package_from_dir(&dir, &pm_str) {
                        packages.push(pkg);
                    }
                }
            }
        }

        // If not a monorepo or root package has its own buildable identity
        if let Some(ref manifest) = root_manifest {
            let root_name = manifest
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("root")
                .to_string();
            let already_included = packages.iter().any(|p| p.name == root_name);
            if !already_included {
                let scripts = extract_scripts(manifest);
                let deps = extract_dep_keys(manifest, "dependencies");
                let dev_deps = extract_dep_keys(manifest, "devDependencies");
                let tsconfig = parse_tsconfig(&self.root).map(|(p, _)| {
                    p.strip_prefix(&self.root)
                        .unwrap_or(&p)
                        .to_string_lossy()
                        .replace('\\', "/")
                });

                // Only push root if it is not monorepo or has scripts
                if !is_monorepo || !scripts.is_empty() {
                    let framework = detect_framework(&self.root).map(|f| match f {
                        FrameworkKind::Vite => "vite".to_string(),
                        FrameworkKind::NextJs => "nextjs".to_string(),
                    });
                    packages.push(WorkspacePackage {
                        name: root_name,
                        relative_path: ".".to_string(),
                        manifest_path: "package.json".to_string(),
                        package_manager: pm_str.clone(),
                        scripts,
                        dependencies: deps,
                        dev_dependencies: dev_deps,
                        tsconfig_path: tsconfig,
                        framework,
                    });
                }
            }
        }

        // Filter by scope if provided
        if let Some(s) = scope {
            let normalized_scope = s.trim_matches('/').replace('\\', "/");
            packages.retain(|p| {
                p.relative_path == normalized_scope
                    || p.relative_path.starts_with(&format!("{normalized_scope}/"))
            });
        }

        // Sort by relative path for deterministic stability
        packages.sort_by(|a, b| a.name.cmp(&b.name));

        let task_runner = self.detect_task_runner();
        let root_framework = detect_framework(&self.root).map(|f| match f {
            FrameworkKind::Vite => "vite".to_string(),
            FrameworkKind::NextJs => "nextjs".to_string(),
        });

        Ok(WorkspacePackagesResult {
            packages,
            package_manager: pm_str,
            is_monorepo,
            task_runner,
            framework: root_framework,
        })
    }

    pub fn discover_targets(
        &self,
        package_filter: Option<&str>,
        scope: Option<&str>,
    ) -> Result<BuildTargetsResult, String> {
        let pkgs_res = self.discover_packages(scope)?;
        let turbo_config = parse_turbo_json(&self.root);
        let nx_config = parse_nx_json(&self.root);
        let task_runner = self.detect_task_runner();

        let mut targets = Vec::new();

        for pkg in &pkgs_res.packages {
            if let Some(pf) = package_filter {
                if pkg.name != pf {
                    continue;
                }
            }

            let pkg_dir = self.root.join(&pkg.relative_path);
            let nx_project = parse_project_json(&pkg_dir);

            // 1. Turborepo targets
            if let Some(ref tc) = turbo_config {
                let turbo_targets = extract_turbo_targets(tc, &pkg.name);
                for mut tt in turbo_targets {
                    // Enrich with package script if exists
                    if let Some(script) = pkg.scripts.get(&tt.target) {
                        tt.command = Some(format!("turbo run {}", tt.target));
                        let _ = script;
                    }
                    targets.push(tt);
                }
            }

            // 2. Nx targets
            if nx_config.is_some() || nx_project.is_some() {
                let nx_targets =
                    extract_nx_targets(nx_config.as_ref(), nx_project.as_ref(), &pkg.name);
                for nt in nx_targets {
                    if !targets
                        .iter()
                        .any(|t: &BuildTarget| t.package == nt.package && t.target == nt.target)
                    {
                        targets.push(nt);
                    }
                }
            }

            // 3. Package manifest scripts
            for (script_name, script_cmd) in &pkg.scripts {
                if !targets
                    .iter()
                    .any(|t| t.package == pkg.name && t.target == *script_name)
                {
                    let runner = if turbo_config.is_some() {
                        "turbo"
                    } else if nx_config.is_some() {
                        "nx"
                    } else {
                        "package-script"
                    };

                    targets.push(BuildTarget {
                        target: script_name.clone(),
                        package: pkg.name.clone(),
                        runner: runner.to_string(),
                        executor: None,
                        command: Some(script_cmd.clone()),
                        inputs: Vec::new(),
                        outputs: Vec::new(),
                        depends_on: Vec::new(),
                        cacheable: true,
                    });
                }
            }

            // 4. TypeScript project references
            if let Some((_, tsconfig)) = parse_tsconfig(&pkg_dir) {
                if !tsconfig.references.is_empty()
                    && !targets
                        .iter()
                        .any(|t| t.package == pkg.name && t.target == "typecheck")
                {
                    targets.push(BuildTarget {
                        target: "typecheck".to_string(),
                        package: pkg.name.clone(),
                        runner: "tsc".to_string(),
                        executor: None,
                        command: Some("tsc -b".to_string()),
                        inputs: vec!["tsconfig.json".to_string()],
                        outputs: vec!["*.tsbuildinfo".to_string()],
                        depends_on: Vec::new(),
                        cacheable: true,
                    });
                }
            }

            // 5. Frameworks (Vite / Next.js)
            if let Some(framework) = detect_framework(&pkg_dir) {
                if !targets
                    .iter()
                    .any(|t| t.package == pkg.name && t.target == "build")
                {
                    let (runner, cmd) = match framework {
                        FrameworkKind::Vite => ("vite", "vite build"),
                        FrameworkKind::NextJs => ("next", "next build"),
                    };
                    targets.push(BuildTarget {
                        target: "build".to_string(),
                        package: pkg.name.clone(),
                        runner: runner.to_string(),
                        executor: None,
                        command: Some(cmd.to_string()),
                        inputs: Vec::new(),
                        outputs: vec!["dist".to_string(), ".next".to_string()],
                        depends_on: Vec::new(),
                        cacheable: true,
                    });
                }
            }
        }

        // Sort targets deterministically by package, then target name
        targets.sort_by(|a, b| match a.package.cmp(&b.package) {
            std::cmp::Ordering::Equal => a.target.cmp(&b.target),
            other => other,
        });

        Ok(BuildTargetsResult {
            targets,
            task_runner,
        })
    }

    pub fn build_dependencies(
        &self,
        package_name: &str,
        target_name: Option<&str>,
    ) -> Result<BuildDependenciesResult, String> {
        self.validate_package_name(package_name)?;
        let pkgs_res = self.discover_packages(None)?;
        let all_pkg_names: HashSet<&str> =
            pkgs_res.packages.iter().map(|p| p.name.as_str()).collect();

        let current_pkg = pkgs_res
            .packages
            .iter()
            .find(|p| p.name == package_name)
            .ok_or_else(|| format!("package '{package_name}' not found in workspace"))?;

        // 1. Direct workspace dependencies
        let mut direct_deps = Vec::new();
        for dep in current_pkg
            .dependencies
            .iter()
            .chain(&current_pkg.dev_dependencies)
        {
            if all_pkg_names.contains(dep.as_str()) && !direct_deps.contains(dep) {
                direct_deps.push(dep.clone());
            }
        }
        direct_deps.sort();

        // 2. Direct workspace dependents (who depends on current_pkg?)
        let mut dependents = Vec::new();
        for other in &pkgs_res.packages {
            if other.name == package_name {
                continue;
            }
            if other.dependencies.contains(&package_name.to_string())
                || other.dev_dependencies.contains(&package_name.to_string())
            {
                dependents.push(other.name.clone());
            }
        }
        dependents.sort();

        // 3. Project references
        let mut project_refs = Vec::new();
        let pkg_dir = self.root.join(&current_pkg.relative_path);
        if let Some((_, tsconfig)) = parse_tsconfig(&pkg_dir) {
            for ref_entry in tsconfig.references {
                let ref_path = pkg_dir.join(&ref_entry.path);
                if let Ok(canon_ref) = ref_path.canonicalize() {
                    for other in &pkgs_res.packages {
                        let other_dir = self.root.join(&other.relative_path);
                        if let Ok(canon_other) = other_dir.canonicalize() {
                            if canon_ref == canon_other
                                && other.name != package_name
                                && !project_refs.contains(&other.name)
                            {
                                project_refs.push(other.name.clone());
                            }
                        }
                    }
                }
            }
        }
        project_refs.sort();

        // 4. Pipeline task dependencies (e.g. ^build from turbo/nx)
        let mut pipeline_deps = Vec::new();
        let target = target_name.unwrap_or("build");
        if let Some(turbo) = parse_turbo_json(&self.root) {
            let tasks = if !turbo.tasks.is_empty() {
                &turbo.tasks
            } else {
                &turbo.pipeline
            };
            if let Some(task) = tasks.get(target) {
                for dep in &task.depends_on {
                    pipeline_deps.push(dep.clone());
                }
            }
        }
        if let Some(nx) = parse_nx_json(&self.root) {
            if let Some(def) = nx.target_defaults.get(target) {
                for dep in &def.depends_on {
                    if !pipeline_deps.contains(dep) {
                        pipeline_deps.push(dep.clone());
                    }
                }
            }
        }
        pipeline_deps.sort();

        Ok(BuildDependenciesResult {
            package: package_name.to_string(),
            dependencies: direct_deps,
            dependents,
            project_references: project_refs,
            pipeline_dependencies: pipeline_deps,
        })
    }

    pub fn build_affected(
        &self,
        files: Option<&[String]>,
        packages: Option<&[String]>,
        target: Option<&str>,
    ) -> Result<BuildAffectedResult, String> {
        let pkgs_res = self.discover_packages(None)?;
        let mut changed_inputs = Vec::new();
        let mut directly_affected_set = BTreeSet::new();

        // 1. Resolve files to packages
        if let Some(file_list) = files {
            for f in file_list {
                self.validate_file_path(f)?;
                changed_inputs.push(f.clone());
                let normalized = f.replace('\\', "/");
                let mut best_match: Option<&WorkspacePackage> = None;

                for pkg in &pkgs_res.packages {
                    let rel = &pkg.relative_path;
                    if rel == "." {
                        if best_match.is_none() {
                            best_match = Some(pkg);
                        }
                    } else if (normalized == *rel || normalized.starts_with(&format!("{rel}/")))
                        && best_match.map_or(true, |bm| bm.relative_path.len() < rel.len())
                    {
                        best_match = Some(pkg);
                    }
                }

                if let Some(pkg) = best_match {
                    directly_affected_set.insert(pkg.name.clone());
                }
            }
        }

        // 2. Add directly specified packages
        if let Some(pkg_list) = packages {
            for p in pkg_list {
                self.validate_package_name(p)?;
                if pkgs_res.packages.iter().any(|pkg| pkg.name == *p) {
                    directly_affected_set.insert(p.clone());
                    changed_inputs.push(format!("package:{p}"));
                }
            }
        }

        // If nothing specified, return empty affected
        if directly_affected_set.is_empty() {
            return Ok(BuildAffectedResult {
                changed_inputs,
                directly_affected: Vec::new(),
                transitive_affected: Vec::new(),
                all_affected_packages: Vec::new(),
                affected_packages: Vec::new(),
                affected_targets: Vec::new(),
                reverse_dependency_paths: BTreeMap::new(),
            });
        }

        // 3. Build reverse dependency graph (child -> all parents that depend on child)
        let mut rev_graph: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for pkg in &pkgs_res.packages {
            for dep in pkg.dependencies.iter().chain(&pkg.dev_dependencies) {
                rev_graph
                    .entry(dep.clone())
                    .or_default()
                    .push(pkg.name.clone());
            }
        }

        // Also add project reference reverse dependencies
        for pkg in &pkgs_res.packages {
            let pkg_dir = self.root.join(&pkg.relative_path);
            if let Some((_, tsconfig)) = parse_tsconfig(&pkg_dir) {
                for ref_entry in tsconfig.references {
                    let ref_path = pkg_dir.join(&ref_entry.path);
                    if let Ok(canon_ref) = ref_path.canonicalize() {
                        for other in &pkgs_res.packages {
                            let other_dir = self.root.join(&other.relative_path);
                            if let Ok(canon_other) = other_dir.canonicalize() {
                                if canon_ref == canon_other && other.name != pkg.name {
                                    let entry = rev_graph.entry(other.name.clone()).or_default();
                                    if !entry.contains(&pkg.name) {
                                        entry.push(pkg.name.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // 4. Reverse dependency traversal (BFS to find all downstream packages)
        let mut transitive_affected_set = BTreeSet::new();
        let mut rev_paths: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut queue: VecDeque<(String, Vec<String>)> = VecDeque::new();

        for direct in &directly_affected_set {
            queue.push_back((direct.clone(), vec![direct.clone()]));
        }

        while let Some((curr, path)) = queue.pop_front() {
            if let Some(parents) = rev_graph.get(&curr) {
                for parent in parents {
                    let mut new_path = path.clone();
                    new_path.push(parent.clone());

                    if !directly_affected_set.contains(parent)
                        && !transitive_affected_set.contains(parent)
                    {
                        transitive_affected_set.insert(parent.clone());
                        rev_paths.insert(parent.clone(), new_path.clone());
                        queue.push_back((parent.clone(), new_path));
                    }
                }
            }
        }

        let directly_affected: Vec<String> = directly_affected_set.into_iter().collect();
        let transitive_affected: Vec<String> = transitive_affected_set.into_iter().collect();

        let mut all_affected = directly_affected.clone();
        for t in &transitive_affected {
            if !all_affected.contains(t) {
                all_affected.push(t.clone());
            }
        }

        // 5. Filter affected targets
        let all_targets = self.discover_targets(None, None)?;
        let target_filter = target.unwrap_or("build");
        let mut affected_targets = Vec::new();

        for t in all_targets.targets {
            if all_affected.contains(&t.package) && (t.target == target_filter || target.is_none())
            {
                affected_targets.push(t);
            }
        }

        Ok(BuildAffectedResult {
            changed_inputs,
            directly_affected,
            transitive_affected,
            all_affected_packages: all_affected.clone(),
            affected_packages: all_affected,
            affected_targets,
            reverse_dependency_paths: rev_paths,
        })
    }

    pub fn build_command(
        &self,
        packages: Option<&[String]>,
        target: Option<&str>,
        files: Option<&[String]>,
    ) -> Result<BuildCommandResult, String> {
        let task_target = target.unwrap_or("build");

        // If packages not directly passed, resolve from affected files
        let effective_packages = if let Some(pkgs) = packages {
            if !pkgs.is_empty() {
                for p in pkgs {
                    self.validate_package_name(p)?;
                }
                pkgs.to_vec()
            } else if files.is_some() {
                let aff = self.build_affected(files, None, Some(task_target))?;
                aff.all_affected_packages
            } else {
                Vec::new()
            }
        } else if files.is_some() {
            let aff = self.build_affected(files, None, Some(task_target))?;
            aff.all_affected_packages
        } else {
            Vec::new()
        };

        let pm = detect_package_manager(&self.root);
        let runner_kind = self.detect_task_runner();

        match runner_kind.as_deref() {
            Some("turbo") => {
                let mut argv = vec![
                    "turbo".to_string(),
                    "run".to_string(),
                    task_target.to_string(),
                ];
                for pkg in &effective_packages {
                    argv.push(format!("--filter={pkg}..."));
                }
                let program = match pm {
                    PackageManager::Pnpm => "pnpm".to_string(),
                    PackageManager::Bun => "bunx".to_string(),
                    PackageManager::Yarn => "yarn".to_string(),
                    PackageManager::Npm => "npx".to_string(),
                };
                let mut full_argv = vec![program.clone()];
                full_argv.extend(argv.clone());
                let raw_cmd = full_argv.join(" ");

                Ok(BuildCommandResult {
                    program,
                    argv,
                    cwd: ".".to_string(),
                    runner: "turbo".to_string(),
                    targets: effective_packages,
                    cache_hint:
                        "Turborepo native caching enabled (task inputs, outputs, and hashes cached)"
                            .to_string(),
                    raw_command: raw_cmd.clone(),
                    command: raw_cmd,
                    supports_cache: true,
                })
            }
            Some("nx") => {
                let mut argv = vec![
                    "nx".to_string(),
                    "run-many".to_string(),
                    "-t".to_string(),
                    task_target.to_string(),
                ];
                if !effective_packages.is_empty() {
                    argv.push(format!("-p={}", effective_packages.join(",")));
                }
                let program = match pm {
                    PackageManager::Pnpm => "pnpm".to_string(),
                    PackageManager::Bun => "bunx".to_string(),
                    PackageManager::Yarn => "yarn".to_string(),
                    PackageManager::Npm => "npx".to_string(),
                };
                let mut full_argv = vec![program.clone()];
                full_argv.extend(argv.clone());
                let raw_cmd = full_argv.join(" ");

                Ok(BuildCommandResult {
                    program,
                    argv,
                    cwd: ".".to_string(),
                    runner: "nx".to_string(),
                    targets: effective_packages,
                    cache_hint:
                        "Nx computational cache enabled (project target outputs and inputs cached)"
                            .to_string(),
                    raw_command: raw_cmd.clone(),
                    command: raw_cmd,
                    supports_cache: true,
                })
            }
            _ => {
                // pnpm workspace or standard package manager runner
                if pm == PackageManager::Pnpm && !effective_packages.is_empty() {
                    let mut argv = vec!["pnpm".to_string()];
                    for pkg in &effective_packages {
                        argv.push(format!("--filter={pkg}..."));
                    }
                    argv.push("run".to_string());
                    argv.push(task_target.to_string());
                    let raw_cmd = argv.join(" ");

                    Ok(BuildCommandResult {
                        program: "pnpm".to_string(),
                        argv,
                        cwd: ".".to_string(),
                        runner: "pnpm".to_string(),
                        targets: effective_packages,
                        cache_hint:
                            "pnpm workspace execution with hard-linked store and tsbuildinfo cache"
                                .to_string(),
                        raw_command: raw_cmd.clone(),
                        command: raw_cmd,
                        supports_cache: true,
                    })
                } else {
                    let program = pm.as_str().to_string();
                    let argv = pm.run_script_argv(task_target);
                    let raw_cmd = argv.join(" ");

                    Ok(BuildCommandResult {
                        program,
                        argv,
                        cwd: ".".to_string(),
                        runner: "package-script".to_string(),
                        targets: effective_packages,
                        cache_hint:
                            "Standard package-manager script with compiler tsbuildinfo cache"
                                .to_string(),
                        raw_command: raw_cmd.clone(),
                        command: raw_cmd,
                        supports_cache: false,
                    })
                }
            }
        }
    }

    fn detect_task_runner(&self) -> Option<String> {
        if self.root.join("turbo.json").is_file() {
            Some("turbo".to_string())
        } else if self.root.join("nx.json").is_file() {
            Some("nx".to_string())
        } else {
            None
        }
    }

    fn load_package_from_dir(&self, dir: &Path, pm: &str) -> Option<WorkspacePackage> {
        let pkg_json = dir.join("package.json");
        if !pkg_json.is_file() {
            return None;
        }
        let content = std::fs::read_to_string(&pkg_json).ok()?;
        let manifest: serde_json::Value = serde_json::from_str(&content).ok()?;
        let name = manifest.get("name").and_then(|n| n.as_str())?.to_string();
        let rel_path = dir
            .strip_prefix(&self.root)
            .unwrap_or(dir)
            .to_string_lossy()
            .replace('\\', "/");
        let manifest_rel = pkg_json
            .strip_prefix(&self.root)
            .unwrap_or(&pkg_json)
            .to_string_lossy()
            .replace('\\', "/");

        let scripts = extract_scripts(&manifest);
        let dependencies = extract_dep_keys(&manifest, "dependencies");
        let dev_dependencies = extract_dep_keys(&manifest, "devDependencies");
        let tsconfig_path = parse_tsconfig(dir).map(|(p, _)| {
            p.strip_prefix(&self.root)
                .unwrap_or(&p)
                .to_string_lossy()
                .replace('\\', "/")
        });

        let framework = detect_framework(dir).map(|f| match f {
            FrameworkKind::Vite => "vite".to_string(),
            FrameworkKind::NextJs => "nextjs".to_string(),
        });

        Some(WorkspacePackage {
            name,
            relative_path: rel_path,
            manifest_path: manifest_rel,
            package_manager: pm.to_string(),
            scripts,
            dependencies,
            dev_dependencies,
            tsconfig_path,
            framework,
        })
    }

    fn match_workspace_glob(&self, glob: &str) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        let normalized = glob.replace('\\', "/");
        let parts: Vec<&str> = normalized.split('/').collect();

        if parts.is_empty() {
            return dirs;
        }

        // Handle standard patterns like "packages/*", "apps/*", "libs/*"
        if parts.len() == 2 && parts[1] == "*" {
            let base_dir = self.root.join(parts[0]);
            if base_dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&base_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_dir() && path.join("package.json").is_file() {
                            dirs.push(path);
                        }
                    }
                }
            }
        } else {
            // Direct path
            let dir = self.root.join(&normalized);
            if dir.is_dir() && dir.join("package.json").is_file() {
                dirs.push(dir);
            }
        }

        dirs.sort();
        dirs
    }

    fn validate_package_name(&self, name: &str) -> Result<(), String> {
        if name.is_empty() {
            return Err("package name cannot be empty".into());
        }
        if name.contains("..") || name.starts_with('/') || name.contains('\\') {
            return Err(format!(
                "invalid package name '{name}': path traversal rejected"
            ));
        }
        Ok(())
    }

    fn validate_scope(&self, scope: Option<&str>) -> Result<(), String> {
        if let Some(s) = scope {
            if s.contains("..") || s.starts_with('/') || s.contains('\\') {
                return Err(format!("invalid scope '{s}': path traversal rejected"));
            }
        }
        Ok(())
    }

    fn validate_file_path(&self, file_path: &str) -> Result<(), String> {
        if file_path.contains("..") || file_path.starts_with('/') || file_path.contains('\\') {
            return Err(format!(
                "invalid file path '{file_path}': path traversal rejected"
            ));
        }
        Ok(())
    }
}

fn extract_scripts(manifest: &serde_json::Value) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    if let Some(scripts) = manifest.get("scripts").and_then(|s| s.as_object()) {
        for (k, v) in scripts {
            if let Some(s) = v.as_str() {
                map.insert(k.clone(), s.to_string());
            }
        }
    }
    map
}

fn extract_dep_keys(manifest: &serde_json::Value, field: &str) -> Vec<String> {
    let mut list = Vec::new();
    if let Some(deps) = manifest.get(field).and_then(|d| d.as_object()) {
        for k in deps.keys() {
            list.push(k.clone());
        }
    }
    list.sort();
    list
}
