use serde::{Deserialize, Serialize};
use serde_json::json;

pub const TOOL_NAMES: &[&str] = &[
    "git_symbol_history",
    "git_related_commits",
    "git_changed_symbols",
    "git_branch_diff",
    "git_blame_symbol",
    "git_commit_context",
    "git_conflict_explain",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitSymbolHistoryArgs {
    pub symbol: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default, rename = "maxCommits")]
    pub max_commits: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitRelatedCommitsArgs {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitChangedSymbolsArgs {
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub head: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitBranchDiffArgs {
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub head: Option<String>,
    #[serde(default, rename = "statOnly")]
    pub stat_only: Option<bool>,
    #[serde(default, rename = "maxFiles")]
    pub max_files: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitBlameSymbolArgs {
    pub symbol: String,
    pub path: String,
    #[serde(default)]
    pub revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitCommitContextArgs {
    pub commit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitConflictExplainArgs {
    #[serde(default)]
    pub path: Option<String>,
}

pub fn tool_spec(name: &str) -> Option<davinci_ai::ToolSpec> {
    match name {
        "git_symbol_history" => Some(davinci_ai::ToolSpec {
            name: "git_symbol_history".into(),
            description: "Trace the evolution and introduction of a specific symbol across Git commits, tracking file movements and separating factual line edits from message inference.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "symbol": {
                        "type": "string",
                        "description": "Symbol name or qualified identifier (e.g. 'AuthService.login' or 'login')"
                    },
                    "path": {
                        "type": "string",
                        "description": "Optional file path containing the symbol. If omitted, will be located in repository source files."
                    },
                    "maxCommits": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 50,
                        "description": "Maximum number of commits to inspect (default: 20)"
                    }
                },
                "required": ["symbol"]
            }),
            constrained_sampling: None,
        }),
        "git_related_commits" => Some(davinci_ai::ToolSpec {
            name: "git_related_commits".into(),
            description: "Find commits related to a query, file path, or symbol, with separated facts and inference including extracted PR references.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query matching commit messages or topic"
                    },
                    "path": {
                        "type": "string",
                        "description": "File path to restrict commit history"
                    },
                    "symbol": {
                        "type": "string",
                        "description": "Symbol name to look for in commits"
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 50,
                        "description": "Maximum number of commits to return (default: 10)"
                    }
                }
            }),
            constrained_sampling: None,
        }),
        "git_changed_symbols" => Some(davinci_ai::ToolSpec {
            name: "git_changed_symbols".into(),
            description: "Compare two Git revisions (or worktree) and identify added, modified, and deleted code symbols using tree-sitter AST analysis.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "base": {
                        "type": "string",
                        "description": "Base revision (commit SHA, branch, or 'HEAD~1')"
                    },
                    "head": {
                        "type": "string",
                        "description": "Head revision (commit SHA, branch, or 'HEAD')"
                    },
                    "path": {
                        "type": "string",
                        "description": "Optional file or directory path filter"
                    }
                }
            }),
            constrained_sampling: None,
        }),
        "git_branch_diff" => Some(davinci_ai::ToolSpec {
            name: "git_branch_diff".into(),
            description: "Inspect branch diff against base branch (e.g. main), calculating merge-base, commits ahead/behind, file stats and bounded patches.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "base": {
                        "type": "string",
                        "description": "Base branch or ref (default: 'main')"
                    },
                    "head": {
                        "type": "string",
                        "description": "Head branch or ref (default: 'HEAD')"
                    },
                    "statOnly": {
                        "type": "boolean",
                        "description": "If true, return only file stat summaries without diff snippets"
                    },
                    "maxFiles": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 100,
                        "description": "Maximum files to include in diff (default: 50)"
                    }
                }
            }),
            constrained_sampling: None,
        }),
        "git_blame_symbol" => Some(davinci_ai::ToolSpec {
            name: "git_blame_symbol".into(),
            description: "Attribute every line of a code symbol's range to exact commits and authors using porcelain blame.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "symbol": {
                        "type": "string",
                        "description": "Symbol name to blame"
                    },
                    "path": {
                        "type": "string",
                        "description": "File path containing the symbol"
                    },
                    "revision": {
                        "type": "string",
                        "description": "Git revision (default: 'HEAD')"
                    }
                },
                "required": ["symbol", "path"]
            }),
            constrained_sampling: None,
        }),
        "git_commit_context" => Some(davinci_ai::ToolSpec {
            name: "git_commit_context".into(),
            description: "Inspect detailed commit metadata, parent commits, diff stats, modified symbols, and extracted PR references.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "commit": {
                        "type": "string",
                        "description": "Commit SHA or reference (e.g. 'HEAD', 'HEAD~1', or 40-char SHA)"
                    }
                },
                "required": ["commit"]
            }),
            constrained_sampling: None,
        }),
        "git_conflict_explain" => Some(davinci_ai::ToolSpec {
            name: "git_conflict_explain".into(),
            description: "Explain 3-way merge conflict stages (base, ours, theirs) and overlapping code symbols without mutating files or repository state.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Optional file path to inspect conflict for. If omitted, inspects all unmerged files."
                    }
                }
            }),
            constrained_sampling: None,
        }),
        _ => None,
    }
}
