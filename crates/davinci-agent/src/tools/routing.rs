//! Conservative, deterministic schema routing for explicitly relevant tasks.
//! Routing returns family IDs; the caller intersects them with live authority.

use std::collections::BTreeSet;

use crate::runtime::RuntimeCapability;

pub(crate) fn relevant_tool_families(
    request: &str,
    capabilities: &[RuntimeCapability],
) -> Vec<String> {
    let request = request.to_lowercase();
    let words = request
        .split(|ch: char| !ch.is_alphanumeric() && !matches!(ch, '_' | '-' | ':'))
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    let mentions = |phrase: &str| {
        let phrase = phrase.split_whitespace().collect::<Vec<_>>();
        words.windows(phrase.len()).any(|window| window == phrase)
    };
    let rules: &[(&str, &[&str])] = &[
        (
            "browser",
            &[
                "browser",
                "playwright",
                "puppeteer",
                "screenshot",
                "webpage",
            ],
        ),
        (
            "git",
            &[
                "git",
                "commit history",
                "commits",
                "blame",
                "rebase",
                "branch diff",
            ],
        ),
        (
            "lsp",
            &[
                "lsp",
                "language server",
                "go to definition",
                "find references",
                "rename symbol",
            ],
        ),
        (
            "package",
            &[
                "npm",
                "pnpm",
                "yarn",
                "package exports",
                "package dependencies",
                "package dependents",
            ],
        ),
        (
            "process",
            &[
                "background process",
                "running process",
                "start server",
                "dev server",
                "development server",
            ],
        ),
        (
            "build",
            &[
                "build targets",
                "build dependencies",
                "build graph",
                "monorepo",
            ],
        ),
        (
            "sec",
            &[
                "security audit",
                "security scan",
                "vulnerability",
                "vulnerabilities",
                "cve",
            ],
        ),
        ("graph", &["graph workflow", "workflow graph"]),
        ("tests", &["test impact", "impacted tests", "related tests"]),
        ("impact", &["change impact", "blast radius"]),
        ("verification", &["verification plan"]),
        (
            "repo",
            &[
                "repository map",
                "symbol relationships",
                "file dependencies",
            ],
        ),
        ("workspace", &["workspace checkpoint", "workspace restore"]),
    ];
    let mut families = rules
        .iter()
        .filter(|(_, phrases)| phrases.iter().any(|phrase| mentions(phrase)))
        .map(|(family, _)| (*family).to_owned())
        .collect::<BTreeSet<_>>();

    // Explicit registered tool/family names support new adapter namespaces
    // without inventing family membership from an arbitrary name prefix.
    for capability in capabilities {
        let Some(family) = &capability.family else {
            continue;
        };
        if words.contains(&capability.name.to_lowercase().as_str())
            || (family.contains(':') && words.contains(&family.to_lowercase().as_str()))
        {
            families.insert(family.clone());
        }
    }
    families.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relevance_examples_select_families_without_generic_coding_noise() {
        let cases: &[(&str, &[&str])] = &[
            ("Fix parser.rs, update its tests and build the result", &[]),
            ("Explain the digit and the browserless graph package", &[]),
            ("Inspect the Git commit history", &["git"]),
            ("Use Playwright to take a screenshot", &["browser"]),
            ("Find references with the language server", &["lsp"]),
            ("Check package exports with pnpm", &["package"]),
            ("Start server in a background process", &["process"]),
            ("Inspect monorepo build targets", &["build"]),
            ("Run a security audit for vulnerabilities", &["sec"]),
            ("Execute the graph workflow", &["graph"]),
            (
                "Check change impact and related tests",
                &["impact", "tests"],
            ),
        ];
        for (request, expected) in cases {
            assert_eq!(relevant_tool_families(request, &[]), *expected, "{request}");
        }
    }
}
