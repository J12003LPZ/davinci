//! Data model for P9 Change Impact Engine.
//! Strictly separates direct semantic impact from structural AST impact,
//! and attributes every impact item to its concrete evidence source.

use crate::native_extensions::repo_intelligence::SourceRange;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceSource {
    Lsp,
    Ast,
    Package,
    TestMap,
    Build,
    Git,
    Config,
    Transaction,
}

impl std::fmt::Display for EvidenceSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lsp => write!(f, "LSP"),
            Self::Ast => write!(f, "AST"),
            Self::Package => write!(f, "Package"),
            Self::TestMap => write!(f, "TestMap"),
            Self::Build => write!(f, "Build"),
            Self::Git => write!(f, "Git"),
            Self::Config => write!(f, "Config"),
            Self::Transaction => write!(f, "Transaction"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImpactItem {
    pub name: String,
    pub path: String,
    pub evidence_source: EvidenceSource,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<SourceRange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectSemanticImpact {
    pub items: Vec<ImpactItem>,
    pub lsp_status: String,
    pub reference_count: usize,
    pub summary: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StructuralImpact {
    pub items: Vec<ImpactItem>,
    pub importing_modules: Vec<String>,
    pub imported_modules: Vec<String>,
    pub summary: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestsImpact {
    pub items: Vec<ImpactItem>,
    pub selected_tests: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_command: Option<String>,
    pub broader_verification_required: bool,
    pub summary: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackagesImpact {
    pub items: Vec<ImpactItem>,
    pub affected_packages: Vec<String>,
    pub summary: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildTargetsImpact {
    pub items: Vec<ImpactItem>,
    pub affected_targets: Vec<String>,
    pub summary: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicApiRisk {
    pub items: Vec<ImpactItem>,
    pub is_public_api_affected: bool,
    pub reexports: Vec<String>,
    pub summary: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurationImpact {
    pub items: Vec<ImpactItem>,
    pub is_config_affected: bool,
    pub affected_configs: Vec<String>,
    pub summary: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PotentialBrowserFlows {
    pub items: Vec<ImpactItem>,
    pub affected_flows: Vec<String>,
    pub has_ui_impact: bool,
    pub summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnalysisCompleteness {
    Complete,
    Partial,
    Incomplete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeImpactReport {
    pub files: Vec<String>,
    pub symbols: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<String>,
    pub completeness: AnalysisCompleteness,
    pub confidence: String,
    pub warnings: Vec<String>,
    pub direct_semantic_impact: DirectSemanticImpact,
    pub structural_impact: StructuralImpact,
    pub tests: TestsImpact,
    pub packages: PackagesImpact,
    pub build_targets: BuildTargetsImpact,
    pub public_api_risk: PublicApiRisk,
    pub configuration_impact: ConfigurationImpact,
    pub potential_browser_flows: PotentialBrowserFlows,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeImpactConfig {
    pub enabled: bool,
    pub max_depth: usize,
    pub max_references: usize,
    pub max_files: usize,
}

impl Default for ChangeImpactConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_depth: 5,
            max_references: 50,
            max_files: 64,
        }
    }
}
