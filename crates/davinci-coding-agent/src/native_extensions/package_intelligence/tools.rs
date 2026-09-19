use serde::Deserialize;
use serde_json::json;

pub const TOOL_NAMES: &[&str] = &[
    "package_info",
    "package_exports",
    "package_symbol",
    "package_dependents",
    "package_why",
];

#[derive(Debug, Deserialize)]
pub struct PackageInfoArgs {
    pub package: String,
    #[serde(default)]
    pub workspace: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PackageExportsArgs {
    pub package: String,
    #[serde(default)]
    pub workspace: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PackageSymbolArgs {
    pub package: String,
    pub symbol: String,
    #[serde(default)]
    pub workspace: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PackageDependentsArgs {
    pub package: String,
    #[serde(default)]
    pub workspace: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PackageWhyArgs {
    pub package: String,
    #[serde(default)]
    pub workspace: Option<String>,
}

pub fn tool_spec(name: &str) -> Option<davinci_ai::ToolSpec> {
    match name {
        "package_info" => Some(davinci_ai::ToolSpec {
            name: name.to_string(),
            description: "Inspect installed TypeScript/JavaScript package metadata, declared versions, lock provenance, type declarations and dependencies.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "package": {
                        "type": "string",
                        "description": "The npm/pnpm/yarn package name (e.g. 'zod' or '@types/node')"
                    },
                    "workspace": {
                        "type": "string",
                        "description": "Optional workspace package relative path (e.g. 'packages/api'). Defaults to root."
                    }
                },
                "required": ["package"]
            }),
            constrained_sampling: None,
        }),
        "package_exports" => Some(davinci_ai::ToolSpec {
            name: name.to_string(),
            description: "Inspect package export maps and conditional exports (import, require, types, default) for an installed package.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "package": {
                        "type": "string",
                        "description": "The package name to inspect exports for"
                    },
                    "workspace": {
                        "type": "string",
                        "description": "Optional workspace package relative path. Defaults to root."
                    }
                },
                "required": ["package"]
            }),
            constrained_sampling: None,
        }),
        "package_symbol" => Some(davinci_ai::ToolSpec {
            name: name.to_string(),
            description: "Selectively inspect installed type definitions (.d.ts) or entry points for a qualified symbol (e.g. 'z.object' or function/type name) without whole-tree indexing.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "package": {
                        "type": "string",
                        "description": "The package name (e.g. 'zod')"
                    },
                    "symbol": {
                        "type": "string",
                        "description": "The symbol or qualified member name to inspect (e.g. 'z.object' or 'Chalk')"
                    },
                    "workspace": {
                        "type": "string",
                        "description": "Optional workspace package relative path. Defaults to root."
                    }
                },
                "required": ["package", "symbol"]
            }),
            constrained_sampling: None,
        }),
        "package_dependents" => Some(davinci_ai::ToolSpec {
            name: name.to_string(),
            description: "Find which workspace packages or repository files declare or depend on a given package.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "package": {
                        "type": "string",
                        "description": "The package name to find dependents for"
                    },
                    "workspace": {
                        "type": "string",
                        "description": "Optional workspace package relative path. Defaults to root."
                    }
                },
                "required": ["package"]
            }),
            constrained_sampling: None,
        }),
        "package_why" => Some(davinci_ai::ToolSpec {
            name: name.to_string(),
            description: "Explain why a package is installed in the workspace (direct, transitive, dev, peer, or lockfile dependency reasons).".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "package": {
                        "type": "string",
                        "description": "The package name to investigate"
                    },
                    "workspace": {
                        "type": "string",
                        "description": "Optional workspace package relative path. Defaults to root."
                    }
                },
                "required": ["package"]
            }),
            constrained_sampling: None,
        }),
        _ => None,
    }
}
