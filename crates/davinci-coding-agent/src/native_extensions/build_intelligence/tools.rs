use serde::Deserialize;
use serde_json::json;

pub const TOOL_NAMES: &[&str] = &[
    "workspace_packages",
    "build_targets",
    "build_dependencies",
    "build_affected",
    "build_command",
];

#[derive(Debug, Deserialize)]
pub struct WorkspacePackagesArgs {
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BuildTargetsArgs {
    #[serde(default)]
    pub package: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BuildDependenciesArgs {
    pub package: String,
    #[serde(default)]
    pub target: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BuildAffectedArgs {
    #[serde(default)]
    pub files: Option<Vec<String>>,
    #[serde(default)]
    pub packages: Option<Vec<String>>,
    #[serde(default)]
    pub target: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BuildCommandArgs {
    #[serde(default)]
    pub packages: Option<Vec<String>>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub files: Option<Vec<String>>,
}

pub fn tool_spec(name: &str) -> Option<davinci_ai::ToolSpec> {
    match name {
        "workspace_packages" => Some(davinci_ai::ToolSpec {
            name: "workspace_packages".into(),
            description: "Discover packages in the workspace, package manager, and declared scripts."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "scope": {
                        "type": "string",
                        "description": "Optional workspace-relative subpath to restrict package discovery to"
                    }
                }
            }),
            constrained_sampling: None,
        }),
        "build_targets" => Some(davinci_ai::ToolSpec {
            name: "build_targets".into(),
            description: "Discover build, typecheck, and lint targets from Turborepo, Nx, tsconfig, and package manifests."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "package": {
                        "type": "string",
                        "description": "Optional package name to retrieve targets for"
                    },
                    "scope": {
                        "type": "string",
                        "description": "Optional workspace-relative subpath scope"
                    }
                }
            }),
            constrained_sampling: None,
        }),
        "build_dependencies" => Some(davinci_ai::ToolSpec {
            name: "build_dependencies".into(),
            description: "Inspect direct, transitive, project-reference, and task pipeline dependencies for a package or target."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["package"],
                "properties": {
                    "package": {
                        "type": "string",
                        "description": "Package name to inspect build dependencies for"
                    },
                    "target": {
                        "type": "string",
                        "description": "Optional target name (e.g. build, typecheck)"
                    }
                }
            }),
            constrained_sampling: None,
        }),
        "build_affected" => Some(davinci_ai::ToolSpec {
            name: "build_affected".into(),
            description: "Calculate affected workspace packages and targets based on modified files or packages, with reverse dependency traversal."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "files": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "List of changed file paths relative to the workspace root"
                    },
                    "packages": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "List of directly changed package names"
                    },
                    "target": {
                        "type": "string",
                        "description": "Optional target filter (e.g. build, typecheck)"
                    }
                }
            }),
            constrained_sampling: None,
        }),
        "build_command" => Some(davinci_ai::ToolSpec {
            name: "build_command".into(),
            description: "Suggest repository-native deterministic build/typecheck command without executing it, reporting native cache hints."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "packages": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Target packages to build"
                    },
                    "target": {
                        "type": "string",
                        "description": "Target task to execute (default: build)"
                    },
                    "files": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional modified files to infer affected targets and synthesize filtered command"
                    }
                }
            }),
            constrained_sampling: None,
        }),
        _ => None,
    }
}
