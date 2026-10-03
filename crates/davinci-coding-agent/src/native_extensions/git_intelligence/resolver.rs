use super::{
    model::*,
    runner::{self, run},
};
use crate::native_extensions::repo_intelligence::parse_source;
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
};

type ConflictStageShas = (Option<String>, Option<String>, Option<String>);

pub struct GitResolver {
    root: PathBuf,
    config: GitIntelligenceConfig,
}

impl GitResolver {
    pub fn new(root: &Path, config: GitIntelligenceConfig) -> Result<Self, String> {
        let git_root = runner::git_root(root)?;
        Ok(Self {
            root: git_root,
            config,
        })
    }

    #[allow(dead_code)]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Locate a file containing the symbol if path is not specified.
    pub fn locate_symbol_file(&self, symbol: &str) -> Option<String> {
        // Look in working tree files
        let extensions = ["ts", "tsx", "js", "jsx", "rs", "py", "go"];
        for entry in walkdir::WalkDir::new(&self.root)
            .into_iter()
            .filter_entry(|e| {
                let name = e.file_name().to_string_lossy();
                name != ".git" && name != "node_modules" && name != "target"
            })
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();
            if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                if extensions.contains(&ext) {
                    if let Ok(content) = std::fs::read_to_string(path) {
                        if content.contains(symbol) {
                            if let Ok(rel) = path.strip_prefix(&self.root) {
                                let rel_str = rel.to_string_lossy().replace('\\', "/");
                                if let Ok(repo_file) = parse_source(&rel_str, &content) {
                                    if repo_file.symbols.iter().any(|s| {
                                        s.name == symbol
                                            || s.qualified_name == symbol
                                            || s.name.ends_with(symbol)
                                    }) {
                                        return Some(rel_str);
                                    }
                                }
                                return Some(rel_str);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Read historical blob content at a specific commit and relative path.
    pub fn read_blob_at_commit(&self, commit_sha: &str, rel_path: &str) -> Option<String> {
        let spec = format!("{commit_sha}:{rel_path}");
        let out = run(&self.root, &["cat-file", "blob", &spec]).ok()?;
        String::from_utf8(out).ok()
    }

    /// Extract PR number from a commit message.
    fn extract_pr_number(message: &str) -> Option<u64> {
        let lower = message.to_lowercase();
        // Look for patterns like (#123), PR #123, pull request #123, #123
        for line in lower.lines() {
            if let Some(idx) = line.find('#') {
                let rest = &line[idx + 1..];
                let num_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(num) = num_str.parse::<u64>() {
                    return Some(num);
                }
            }
        }
        None
    }

    /// Extract inferred conventional intent from a commit subject.
    fn extract_intent_summary(subject: &str) -> Option<String> {
        let trimmed = subject.trim();
        if let Some((tag, rest)) = trimmed.split_once(':') {
            let tag = tag.trim().to_lowercase();
            let rest = rest.trim();
            let category = if tag.starts_with("fix") {
                "bug fix"
            } else if tag.starts_with("feat") {
                "new feature"
            } else if tag.starts_with("refactor") {
                "code refactoring"
            } else if tag.starts_with("perf") {
                "performance improvement"
            } else if tag.starts_with("docs") {
                "documentation"
            } else if tag.starts_with("test") {
                "test update"
            } else if tag.starts_with("chore") {
                "maintenance chore"
            } else {
                "modification"
            };
            Some(format!("{category}: {rest}"))
        } else {
            Some(trimmed.to_string())
        }
    }

    // ------------------------------------------------------------------------
    // 1. git_symbol_history
    // ------------------------------------------------------------------------
    pub fn symbol_history(
        &self,
        symbol: &str,
        path: Option<&str>,
        max_commits: Option<usize>,
    ) -> Result<SymbolHistoryResult, String> {
        let max_commits = max_commits.unwrap_or(self.config.max_commits).min(50);
        let target_file = match path {
            Some(p) => {
                let _ = runner::validate_path(&self.root, p)?;
                p.replace('\\', "/")
            }
            None => self
                .locate_symbol_file(symbol)
                .ok_or_else(|| format!("symbol '{symbol}' not found in repository files"))?,
        };

        let is_shallow = runner::is_shallow_repo(&self.root);

        // Fetch bounded log with --follow and name-status
        let limit_str = format!("-n{}", max_commits + 1);
        let out = run(
            &self.root,
            &[
                "log",
                "--follow",
                "--name-status",
                "--format=COMMIT%x00%H%x00%an%x00%ae%x00%aI%x00%s",
                &limit_str,
                "--",
                &target_file,
            ],
        )?;

        let text = String::from_utf8_lossy(&out);
        let mut raw_commits = Vec::new();
        let mut file_movements = Vec::new();

        let mut current_commit_info: Option<(String, String, String, String, String)> = None;
        let mut path_at_commit = target_file.clone();

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix("COMMIT\0") {
                if let Some((sha, author, email, date, summary)) = current_commit_info.take() {
                    raw_commits.push((sha, author, email, date, summary, path_at_commit.clone()));
                }
                let parts: Vec<&str> = rest.split('\0').collect();
                if parts.len() >= 5 {
                    current_commit_info = Some((
                        parts[0].to_string(),
                        parts[1].to_string(),
                        parts[2].to_string(),
                        parts[3].to_string(),
                        parts[4].to_string(),
                    ));
                }
            } else if line.starts_with('R') {
                // Rename line, e.g. R100\told\tnew
                let parts: Vec<&str> = line.split('\t').collect();
                if parts.len() >= 3 {
                    let old_p = parts[1].replace('\\', "/");
                    let new_p = parts[2].replace('\\', "/");
                    if let Some((sha, _, _, _, _)) = &current_commit_info {
                        file_movements.push(FileMovement {
                            from_path: old_p.clone(),
                            to_path: new_p,
                            commit: sha.clone(),
                        });
                    }
                    path_at_commit = old_p;
                }
            }
        }
        if let Some((sha, author, email, date, summary)) = current_commit_info.take() {
            raw_commits.push((sha, author, email, date, summary, path_at_commit.clone()));
        }

        let history_exceeded_limit = raw_commits.len() > max_commits;
        if history_exceeded_limit {
            raw_commits.truncate(max_commits);
        }

        let mut symbol_commits = Vec::new();
        let mut notes = Vec::new();

        // Inspect each commit's blob to see if symbol exists
        for (sha, author, email, date, summary, path_in_rev) in &raw_commits {
            let content = self.read_blob_at_commit(sha, path_in_rev);
            let has_symbol = match &content {
                Some(src) => {
                    if let Ok(repo_file) = parse_source(path_in_rev, src) {
                        repo_file.symbols.iter().any(|s| {
                            s.name == symbol
                                || s.qualified_name == symbol
                                || s.name.ends_with(symbol)
                        })
                    } else {
                        src.contains(symbol)
                    }
                }
                None => false,
            };

            if has_symbol {
                let fact = SymbolCommitFact {
                    commit: sha.clone(),
                    author: author.clone(),
                    author_email: email.clone(),
                    date: date.clone(),
                    summary: summary.clone(),
                    change_kind: "modified".to_string(),
                    path: path_in_rev.clone(),
                    lines_changed: None,
                };
                symbol_commits.push(fact);

                if let Some(intent) = Self::extract_intent_summary(summary) {
                    notes.push(format!(
                        "Commit {} message suggests: {}",
                        &sha[..7.min(sha.len())],
                        intent
                    ));
                }
            }
        }

        // Acceptance guard: An incomplete history must report unknown/partial introduction
        // and coverage rather than claim the first returned commit introduced the symbol.
        let (intro_status, introduced_commit) = if symbol_commits.is_empty() {
            (IntroductionStatus::Unknown, None)
        } else if is_shallow || history_exceeded_limit {
            (IntroductionStatus::Partial, None)
        } else {
            // Check if oldest inspected commit is the true root or if parent has the symbol
            let oldest = symbol_commits.last().unwrap();
            let parent_check = run(&self.root, &["rev-parse", &format!("{}^@", oldest.commit)]);
            let parents = parent_check
                .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_string())
                .unwrap_or_default();

            if parents.is_empty() {
                // Root commit of entire repository
                let mut intro = oldest.clone();
                intro.change_kind = "introduced".to_string();
                (IntroductionStatus::Introduced, Some(intro))
            } else {
                let first_parent = parents.split_whitespace().next().unwrap_or_default();
                let parent_content = self.read_blob_at_commit(first_parent, &oldest.path);
                let parent_had_symbol = match parent_content {
                    Some(src) => {
                        if let Ok(repo_file) = parse_source(&oldest.path, &src) {
                            repo_file.symbols.iter().any(|s| {
                                s.name == symbol
                                    || s.qualified_name == symbol
                                    || s.name.ends_with(symbol)
                            })
                        } else {
                            src.contains(symbol)
                        }
                    }
                    None => false,
                };

                if !parent_had_symbol {
                    let mut intro = oldest.clone();
                    intro.change_kind = "introduced".to_string();
                    (IntroductionStatus::Introduced, Some(intro))
                } else {
                    // Symbol existed before oldest inspected commit
                    (IntroductionStatus::Unknown, None)
                }
            }
        };

        let revisions_with_symbol = symbol_commits.len();
        let commits_inspected = raw_commits.len();

        Ok(SymbolHistoryResult {
            symbol: symbol.to_string(),
            file: target_file,
            introduction_status: intro_status,
            introduced_commit,
            modifications: symbol_commits,
            file_movements,
            facts: SymbolHistoryFacts {
                commits_inspected,
                revisions_with_symbol,
                total_line_modifications: revisions_with_symbol,
                is_shallow,
            },
            inference: SymbolHistoryInference { notes },
        })
    }

    // ------------------------------------------------------------------------
    // 2. git_related_commits
    // ------------------------------------------------------------------------
    pub fn related_commits(
        &self,
        query: Option<&str>,
        path: Option<&str>,
        symbol: Option<&str>,
        limit: Option<usize>,
    ) -> Result<RelatedCommitsResult, String> {
        let limit = limit.unwrap_or(10).min(50);
        let limit_str = format!("-n{limit}");

        let mut args: Vec<&str> = vec![
            "log",
            &limit_str,
            "--format=COMMIT%x00%H%x00%an%x00%ae%x00%aI%x00%s%x00%b",
            "--numstat",
        ];

        let grep_arg: String;
        if let Some(q) = query {
            grep_arg = format!("--grep={q}");
            args.push(&grep_arg);
        } else if let Some(sym) = symbol {
            grep_arg = format!("--grep={sym}");
            args.push(&grep_arg);
        }

        let validated_path: PathBuf;
        if let Some(p) = path {
            validated_path = runner::validate_path(&self.root, p)?;
            let rel = validated_path
                .strip_prefix(&self.root)
                .unwrap()
                .to_str()
                .unwrap();
            args.push("--");
            args.push(rel);
        }

        let out = run(&self.root, &args)?;
        let text = String::from_utf8_lossy(&out);

        let mut commits = Vec::new();
        let mut cur_header: Option<(String, String, String, String, String, String)> = None;
        let mut cur_files = 0usize;
        let mut cur_ins = 0usize;
        let mut cur_del = 0usize;

        let flush = |commits: &mut Vec<RelatedCommit>,
                     header: Option<(String, String, String, String, String, String)>,
                     files: usize,
                     ins: usize,
                     del: usize| {
            if let Some((sha, author, email, date, subject, body)) = header {
                let full_msg = if body.trim().is_empty() {
                    subject.clone()
                } else {
                    format!("{subject}\n\n{body}")
                };
                let pr_number = Self::extract_pr_number(&full_msg);
                let intent = Self::extract_intent_summary(&subject);
                commits.push(RelatedCommit {
                    commit: sha,
                    author,
                    author_email: email,
                    date,
                    message: full_msg,
                    facts: CommitFacts {
                        files_changed: files,
                        insertions: ins,
                        deletions: del,
                    },
                    inference: CommitInference {
                        pr_number,
                        intent_summary: intent,
                    },
                });
            }
        };

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix("COMMIT\0") {
                flush(&mut commits, cur_header.take(), cur_files, cur_ins, cur_del);
                cur_files = 0;
                cur_ins = 0;
                cur_del = 0;

                let parts: Vec<&str> = rest.split('\0').collect();
                if parts.len() >= 6 {
                    cur_header = Some((
                        parts[0].to_string(),
                        parts[1].to_string(),
                        parts[2].to_string(),
                        parts[3].to_string(),
                        parts[4].to_string(),
                        parts[5].to_string(),
                    ));
                }
            } else {
                // numstat line: added \t deleted \t file
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 3 {
                    cur_files += 1;
                    if let Ok(i) = parts[0].parse::<usize>() {
                        cur_ins += i;
                    }
                    if let Ok(d) = parts[1].parse::<usize>() {
                        cur_del += d;
                    }
                }
            }
        }
        flush(&mut commits, cur_header.take(), cur_files, cur_ins, cur_del);

        Ok(RelatedCommitsResult {
            query: query.map(str::to_string),
            path: path.map(str::to_string),
            symbol: symbol.map(str::to_string),
            total_commits: commits.len(),
            commits,
        })
    }

    // ------------------------------------------------------------------------
    // 3. git_changed_symbols
    // ------------------------------------------------------------------------
    pub fn changed_symbols(
        &self,
        base: Option<&str>,
        head: Option<&str>,
        path: Option<&str>,
    ) -> Result<ChangedSymbolsResult, String> {
        let base_rev = match base {
            Some(b) => runner::resolve_commit(&self.root, b)?,
            None => {
                // Default to HEAD~1 or HEAD if HEAD~1 fails
                runner::resolve_commit(&self.root, "HEAD~1")
                    .or_else(|_| runner::resolve_commit(&self.root, "HEAD"))?
            }
        };

        let head_rev = match head {
            Some(h) => runner::resolve_commit(&self.root, h)?,
            None => runner::resolve_commit(&self.root, "HEAD")?,
        };

        let mut diff_args = vec![
            "diff",
            "--name-status",
            "--no-renames",
            &base_rev,
            &head_rev,
        ];
        let validated_path: PathBuf;
        if let Some(p) = path {
            validated_path = runner::validate_path(&self.root, p)?;
            let rel = validated_path
                .strip_prefix(&self.root)
                .unwrap()
                .to_str()
                .unwrap();
            diff_args.push("--");
            diff_args.push(rel);
        }

        let out = run(&self.root, &diff_args)?;
        let text = String::from_utf8_lossy(&out);

        let mut changed_symbols = Vec::new();
        let supported_exts = ["ts", "tsx", "js", "jsx", "rs", "py", "go"];

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 2 {
                continue;
            }
            let status = parts[0];
            let file_path = parts[1].replace('\\', "/");

            let ext = Path::new(&file_path)
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            if !supported_exts.contains(&ext) {
                continue;
            }

            let base_content = if status != "A" {
                self.read_blob_at_commit(&base_rev, &file_path)
                    .unwrap_or_default()
            } else {
                String::new()
            };

            let head_content = if status != "D" {
                self.read_blob_at_commit(&head_rev, &file_path)
                    .unwrap_or_default()
            } else {
                String::new()
            };

            let base_file = parse_source(&file_path, &base_content).ok();
            let head_file = parse_source(&file_path, &head_content).ok();

            let base_symbols: HashMap<String, _> = base_file
                .map(|f| f.symbols.into_iter().map(|s| (s.name.clone(), s)).collect())
                .unwrap_or_default();

            let head_symbols: HashMap<String, _> = head_file
                .map(|f| f.symbols.into_iter().map(|s| (s.name.clone(), s)).collect())
                .unwrap_or_default();

            // Added
            for (name, head_sym) in &head_symbols {
                if !base_symbols.contains_key(name) {
                    changed_symbols.push(ChangedSymbol {
                        name: head_sym.name.clone(),
                        qualified_name: head_sym.qualified_name.clone(),
                        kind: head_sym.kind.clone(),
                        file: file_path.clone(),
                        change_type: "added".to_string(),
                        base_range: None,
                        head_range: Some(ChangedSymbolRange {
                            start_line: head_sym.range.start_line,
                            end_line: head_sym.range.end_line,
                        }),
                    });
                }
            }

            // Deleted
            for (name, base_sym) in &base_symbols {
                if !head_symbols.contains_key(name) {
                    changed_symbols.push(ChangedSymbol {
                        name: base_sym.name.clone(),
                        qualified_name: base_sym.qualified_name.clone(),
                        kind: base_sym.kind.clone(),
                        file: file_path.clone(),
                        change_type: "deleted".to_string(),
                        base_range: Some(ChangedSymbolRange {
                            start_line: base_sym.range.start_line,
                            end_line: base_sym.range.end_line,
                        }),
                        head_range: None,
                    });
                }
            }

            // Modified
            for (name, head_sym) in &head_symbols {
                if let Some(base_sym) = base_symbols.get(name) {
                    if base_sym.range.start_line != head_sym.range.start_line
                        || base_sym.range.end_line != head_sym.range.end_line
                        || base_sym.signature != head_sym.signature
                    {
                        changed_symbols.push(ChangedSymbol {
                            name: head_sym.name.clone(),
                            qualified_name: head_sym.qualified_name.clone(),
                            kind: head_sym.kind.clone(),
                            file: file_path.clone(),
                            change_type: "modified".to_string(),
                            base_range: Some(ChangedSymbolRange {
                                start_line: base_sym.range.start_line,
                                end_line: base_sym.range.end_line,
                            }),
                            head_range: Some(ChangedSymbolRange {
                                start_line: head_sym.range.start_line,
                                end_line: head_sym.range.end_line,
                            }),
                        });
                    }
                }
            }
        }

        Ok(ChangedSymbolsResult {
            base: base_rev,
            head: head_rev,
            total_changed_symbols: changed_symbols.len(),
            symbols: changed_symbols,
        })
    }

    // ------------------------------------------------------------------------
    // 4. git_branch_diff
    // ------------------------------------------------------------------------
    pub fn branch_diff(
        &self,
        base: Option<&str>,
        head: Option<&str>,
        stat_only: Option<bool>,
        max_files: Option<usize>,
    ) -> Result<BranchDiffResult, String> {
        let max_files = max_files.unwrap_or(50).min(100);
        let base_name = base.unwrap_or("main");
        let head_name = head.unwrap_or("HEAD");

        let base_sha = runner::resolve_commit(&self.root, base_name)
            .or_else(|_| runner::resolve_commit(&self.root, "master"))
            .unwrap_or_else(|_| base_name.to_string());
        let head_sha = runner::resolve_commit(&self.root, head_name)?;

        // Compute merge-base if possible
        let merge_base = run(&self.root, &["merge-base", &base_sha, &head_sha])
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let (ahead, behind) = if let Some(mb) = &merge_base {
            let ahead_cnt = run(
                &self.root,
                &["rev-list", "--count", &format!("{mb}..{head_sha}")],
            )
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
            .and_then(|s| s.trim().parse::<usize>().ok())
            .unwrap_or(0);
            let behind_cnt = run(
                &self.root,
                &["rev-list", "--count", &format!("{mb}..{base_sha}")],
            )
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
            .and_then(|s| s.trim().parse::<usize>().ok())
            .unwrap_or(0);
            (ahead_cnt, behind_cnt)
        } else {
            (0, 0)
        };

        // Diff stats
        let out = run(&self.root, &["diff", "--numstat", &base_sha, &head_sha])?;
        let text = String::from_utf8_lossy(&out);

        let mut files = Vec::new();
        let mut total_ins = 0usize;
        let mut total_del = 0usize;

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                let ins: usize = parts[0].parse().unwrap_or(0);
                let del: usize = parts[1].parse().unwrap_or(0);
                let path = parts[2].replace('\\', "/");

                total_ins += ins;
                total_del += del;

                let patch_snippet = if !stat_only.unwrap_or(false) && files.len() < max_files {
                    let diff_out = run(
                        &self.root,
                        &["diff", "-U2", &base_sha, &head_sha, "--", &path],
                    )
                    .ok();
                    diff_out.and_then(|b| String::from_utf8(b).ok()).map(|s| {
                        let lines: Vec<&str> = s.lines().take(30).collect();
                        lines.join("\n")
                    })
                } else {
                    None
                };

                if files.len() < max_files {
                    files.push(FileDiffSummary {
                        path,
                        status: "modified".to_string(),
                        insertions: ins,
                        deletions: del,
                        patch_snippet,
                    });
                }
            }
        }

        let files_changed = files.len();
        Ok(BranchDiffResult {
            base: base_sha,
            head: head_sha,
            merge_base,
            commits_ahead: ahead,
            commits_behind: behind,
            files_changed,
            insertions: total_ins,
            deletions: total_del,
            files,
        })
    }

    // ------------------------------------------------------------------------
    // 5. git_blame_symbol
    // ------------------------------------------------------------------------
    pub fn blame_symbol(
        &self,
        symbol: &str,
        path: &str,
        revision: Option<&str>,
    ) -> Result<SymbolBlameResult, String> {
        let _ = runner::validate_path(&self.root, path)?;
        let rev = revision.unwrap_or("HEAD");
        let rev_sha = runner::resolve_commit(&self.root, rev)?;

        let content = self
            .read_blob_at_commit(&rev_sha, path)
            .ok_or_else(|| format!("cannot read file '{path}' at revision '{rev}'"))?;

        let repo_file = parse_source(path, &content)
            .map_err(|e| format!("cannot parse AST for '{path}': {e}"))?;

        let sym = repo_file
            .symbols
            .iter()
            .find(|s| s.name == symbol || s.qualified_name == symbol || s.name.ends_with(symbol))
            .ok_or_else(|| {
                format!("symbol '{symbol}' not found in '{path}' at revision '{rev}'")
            })?;

        let start_line = sym.range.start_line;
        let end_line = sym.range.end_line;
        let line_arg = format!("-L{start_line},{end_line}");

        let out = run(
            &self.root,
            &["blame", &line_arg, "--porcelain", &rev_sha, "--", path],
        )?;

        let text = String::from_utf8_lossy(&out);
        let mut lines = Vec::new();
        let mut author_counts: HashMap<String, usize> = HashMap::new();
        let mut most_recent_commit: Option<String> = None;

        let mut current_sha = String::new();
        let mut current_author = String::new();
        let mut current_date = String::new();
        let mut current_line_num = start_line;

        for raw_line in text.lines() {
            if let Some(content) = raw_line.strip_prefix('\t') {
                // Content line
                lines.push(BlameLine {
                    line_number: current_line_num,
                    commit: current_sha.clone(),
                    author: current_author.clone(),
                    date: current_date.clone(),
                    content: content.to_string(),
                });
                *author_counts.entry(current_author.clone()).or_insert(0) += 1;
                if most_recent_commit.is_none() {
                    most_recent_commit = Some(current_sha.clone());
                }
                current_line_num += 1;
            } else {
                let parts: Vec<&str> = raw_line.split_whitespace().collect();
                if parts.len() >= 4 && (parts[0].len() == 40 || parts[0].len() == 64) {
                    current_sha = parts[0].to_string();
                } else if let Some(author) = raw_line.strip_prefix("author ") {
                    current_author = author.trim().to_string();
                } else if let Some(time) = raw_line.strip_prefix("author-time ") {
                    current_date = time.trim().to_string();
                }
            }
        }

        let total_lines = lines.len();
        let authors = author_counts
            .into_iter()
            .map(|(author, count)| {
                let percentage = if total_lines > 0 {
                    (count as f64 / total_lines as f64) * 100.0
                } else {
                    0.0
                };
                BlameAuthorStat {
                    author,
                    line_count: count,
                    percentage,
                }
            })
            .collect();

        Ok(SymbolBlameResult {
            symbol: symbol.to_string(),
            file: path.to_string(),
            revision: rev_sha,
            start_line,
            end_line,
            total_lines,
            lines,
            authors,
            most_recent_commit,
        })
    }

    // ------------------------------------------------------------------------
    // 6. git_commit_context
    // ------------------------------------------------------------------------
    pub fn commit_context(&self, commit_ref: &str) -> Result<CommitContextResult, String> {
        let sha = runner::resolve_commit(&self.root, commit_ref)?;

        let out = run(
            &self.root,
            &["log", "-1", "--format=%H%x00%an%x00%ae%x00%aI%x00%B", &sha],
        )?;

        let text = String::from_utf8_lossy(&out);
        let parts: Vec<&str> = text.split('\0').collect();
        if parts.len() < 5 {
            return Err(format!("cannot read commit metadata for '{sha}'"));
        }

        let commit_sha = parts[0].to_string();
        let author = parts[1].to_string();
        let author_email = parts[2].to_string();
        let date = parts[3].to_string();
        let message = parts[4].to_string();

        let parent_out = run(&self.root, &["log", "-1", "--format=%P", &sha])?;
        let parents_str = String::from_utf8_lossy(&parent_out);
        let parents: Vec<String> = parents_str.split_whitespace().map(str::to_string).collect();

        // Diff stats
        let diff_out = run(
            &self.root,
            &[
                "diff-tree",
                "--root",
                "--no-commit-id",
                "--numstat",
                "-r",
                &sha,
            ],
        )?;
        let diff_text = String::from_utf8_lossy(&diff_out);

        let mut files_changed = Vec::new();
        let mut total_ins = 0usize;
        let mut total_del = 0usize;

        for line in diff_text.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                if let Ok(i) = parts[0].parse::<usize>() {
                    total_ins += i;
                }
                if let Ok(d) = parts[1].parse::<usize>() {
                    total_del += d;
                }
                files_changed.push(parts[2].replace('\\', "/"));
            }
        }

        // Modified symbols across changed files
        let mut modified_symbols = Vec::new();
        let base_commit = parents.first();
        for f in &files_changed {
            let base_content = base_commit
                .and_then(|p| self.read_blob_at_commit(p, f))
                .unwrap_or_default();
            let head_content = self.read_blob_at_commit(&sha, f).unwrap_or_default();
            if let Ok(hf) = parse_source(f, &head_content) {
                if let Ok(bf) = parse_source(f, &base_content) {
                    let base_map: HashMap<_, _> = bf
                        .symbols
                        .into_iter()
                        .map(|s| (s.name.clone(), s))
                        .collect();
                    for s in hf.symbols {
                        if let Some(bs) = base_map.get(&s.name) {
                            if bs.signature != s.signature || bs.range != s.range {
                                modified_symbols.push(s.name);
                            }
                        } else {
                            modified_symbols.push(s.name);
                        }
                    }
                } else {
                    for s in hf.symbols {
                        modified_symbols.push(s.name);
                    }
                }
            }
        }

        let pr_num = Self::extract_pr_number(&message);
        let pr_reference = pr_num.map(|n| format!("#{n}"));
        let intent_summary = Self::extract_intent_summary(message.lines().next().unwrap_or(""))
            .unwrap_or_else(|| message.clone());

        Ok(CommitContextResult {
            commit: commit_sha,
            author,
            author_email,
            date,
            message,
            parents,
            facts: CommitContextFacts {
                files_changed,
                insertions: total_ins,
                deletions: total_del,
                modified_symbols,
            },
            inference: CommitContextInference {
                pr_reference,
                intent_summary,
            },
        })
    }

    // ------------------------------------------------------------------------
    // 7. git_conflict_explain
    // ------------------------------------------------------------------------
    pub fn conflict_explain(&self, path: Option<&str>) -> Result<ConflictExplainResult, String> {
        let out = run(&self.root, &["ls-files", "--unmerged", "-z"])?;
        let bytes = out.as_slice();

        let mut stages_by_file: BTreeMap<String, ConflictStageShas> = BTreeMap::new();

        for record in bytes.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            let record_str = String::from_utf8_lossy(record);
            if let Some((identity, file_path)) = record_str.split_once('\t') {
                let norm_path = file_path.replace('\\', "/");
                if let Some(target) = path {
                    if norm_path != target.replace('\\', "/") {
                        continue;
                    }
                }
                let fields: Vec<&str> = identity.split_whitespace().collect();
                if fields.len() >= 3 {
                    let obj_id = fields[1].to_string();
                    let stage: u8 = fields[2].parse().unwrap_or(0);
                    let entry = stages_by_file
                        .entry(norm_path)
                        .or_insert((None, None, None));
                    match stage {
                        1 => entry.0 = Some(obj_id),
                        2 => entry.1 = Some(obj_id),
                        3 => entry.2 = Some(obj_id),
                        _ => {}
                    }
                }
            }
        }

        let mut reports = Vec::new();

        for (file_path, (base_obj, ours_obj, theirs_obj)) in stages_by_file {
            let disk_file = self.root.join(&file_path);
            let disk_content = std::fs::read_to_string(&disk_file).unwrap_or_default();

            let mut conflict_markers_count = 0usize;
            let mut conflicting_ranges = Vec::new();

            let mut in_conflict = false;
            let mut start_line = 0usize;
            let mut ours_lines = Vec::new();
            let mut theirs_lines = Vec::new();
            let mut in_theirs = false;

            for (idx, line) in disk_content.lines().enumerate() {
                let line_num = idx + 1;
                if line.starts_with("<<<<<<<") {
                    in_conflict = true;
                    in_theirs = false;
                    start_line = line_num;
                    conflict_markers_count += 1;
                    ours_lines.clear();
                    theirs_lines.clear();
                } else if line.starts_with("=======") && in_conflict {
                    in_theirs = true;
                } else if line.starts_with(">>>>>>>") && in_conflict {
                    in_conflict = false;
                    conflicting_ranges.push(ConflictRange {
                        start_line,
                        end_line: line_num,
                        ours_content: ours_lines.join("\n"),
                        theirs_content: theirs_lines.join("\n"),
                        base_content: None,
                    });
                } else if in_conflict {
                    if in_theirs {
                        theirs_lines.push(line);
                    } else {
                        ours_lines.push(line);
                    }
                }
            }

            // Symbol overlap check
            let mut overlapping_symbols = Vec::new();
            if let Ok(repo_file) = parse_source(&file_path, &disk_content) {
                for sym in repo_file.symbols {
                    for range in &conflicting_ranges {
                        if sym.range.start_line <= range.end_line
                            && sym.range.end_line >= range.start_line
                        {
                            overlapping_symbols.push(sym.name.clone());
                        }
                    }
                }
            }

            reports.push(ConflictFileReport {
                path: file_path,
                has_base_stage: base_obj.is_some(),
                has_ours_stage: ours_obj.is_some(),
                has_theirs_stage: theirs_obj.is_some(),
                base_object: base_obj,
                ours_object: ours_obj,
                theirs_object: theirs_obj,
                conflict_markers_count,
                conflicting_ranges,
                overlapping_symbols,
            });
        }

        let total_conflicted_files = reports.len();
        Ok(ConflictExplainResult {
            total_conflicted_files,
            files: reports,
            zero_mutation_guaranteed: true,
        })
    }
}
