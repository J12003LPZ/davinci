//! Static facts only: no project config imports, scripts, environment or instruction files.
use super::{admission::AuthorizedDesignContext, error::*, skills::byte_hash, store::digest};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignFact {
    pub kind: String,
    pub name: String,
    pub value: String,
    pub path: String,
    pub source_hash: String,
    pub line: u32,
    pub confidence: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignSystemSnapshot {
    pub root: String,
    pub fingerprint: String,
    pub files: BTreeMap<String, String>,
    pub facts: Vec<DesignFact>,
    pub unresolved: Vec<String>,
    pub excluded: Vec<String>,
}
fn allowed_entry(path: &Path) -> bool {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    !name.starts_with('.')
        && ![
            "node_modules",
            "target",
            "dist",
            "build",
            "vendor",
            "coverage",
            "secrets",
            "credentials",
        ]
        .contains(&name.as_str())
        && !name.contains("secret")
        && !name.contains("credential")
        && !name.ends_with(".pem")
        && !name.ends_with(".key")
}
fn literal(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "#.,% -_()/'\"".contains(c))
        && !value.contains("url(")
        && !value.contains("process")
        && !value.contains("env(")
}
pub fn extract_system(ctx: &AuthorizedDesignContext) -> DesignResult<DesignSystemSnapshot> {
    extract_system_at(ctx, ".")
}
pub fn extract_system_at(
    ctx: &AuthorizedDesignContext,
    relative_root: &str,
) -> DesignResult<DesignSystemSnapshot> {
    ctx.check("design_sync", &serde_json::json!({}))?;
    if relative_root != "." {
        super::types::validate_path(relative_root)?;
    }
    let root = ctx.workspace().join(relative_root);
    super::runtime::no_links(&root)?;
    let root = root.canonicalize()?;
    if !root.starts_with(ctx.workspace()) || !root.is_dir() {
        return Err(DesignError::Denied(
            "sync requires a directory inside this workspace".into(),
        ));
    }
    let mut files = BTreeMap::new();
    let mut facts = Vec::new();
    let mut unresolved = Vec::new();
    let mut excluded = Vec::new();
    let mut bytes = 0u64;
    let css = regex::Regex::new(r"(--[A-Za-z][A-Za-z0-9_-]{0,63})\s*:\s*([^;{}]+)")
        .expect("constant CSS pattern");
    let theme = regex::Regex::new(
        r#"\b([A-Za-z][A-Za-z0-9_]{0,63})\s*:\s*["']([#A-Za-z0-9.,% /()_-]{1,128})["']"#,
    )
    .expect("constant theme pattern");
    let props = regex::Regex::new(r"(?:interface|type)\s+([A-Za-z][A-Za-z0-9_]*Props)\b")
        .expect("constant props pattern");
    let walker = ignore::WalkBuilder::new(&root)
        .hidden(true)
        .follow_links(false)
        .require_git(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .max_depth(Some(16))
        .filter_entry(move |entry| entry.depth() == 0 || allowed_entry(entry.path()))
        .sort_by_file_path(|a, b| a.cmp(b))
        .build();
    for (count, entry) in walker.enumerate() {
        ctx.check("design_sync", &serde_json::json!({}))?;
        if count > 10_000 {
            return Err(DesignError::BudgetExceeded(
                "repository scan entry limit".into(),
            ));
        }
        let entry = entry.map_err(|e| DesignError::IoFailure(e.to_string()))?;
        if entry.file_type().is_some_and(|kind| kind.is_dir()) {
            continue;
        }
        let path = entry.path();
        let relative = path
            .strip_prefix(ctx.workspace())
            .map_err(|_| DesignError::Denied("scan escaped workspace".into()))?
            .to_string_lossy()
            .replace('\\', "/");
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            if excluded.len() < 128 {
                excluded.push(relative);
            }
            continue;
        }
        let extension = path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        if ![
            "css", "ts", "tsx", "js", "jsx", "mjs", "cjs", "html", "json",
        ]
        .contains(&extension.as_str())
        {
            continue;
        }
        // JSON is allowed only for the package manifest; other project data may be private.
        if extension == "json" && path.file_name().is_none_or(|name| name != "package.json") {
            continue;
        }
        super::runtime::no_links(path)?;
        ctx.check_static_read(path)?;
        let size = std::fs::metadata(path)?.len();
        if size > 256 * 1024 {
            if excluded.len() < 128 {
                excluded.push(relative);
            }
            continue;
        }
        bytes = bytes
            .checked_add(size)
            .ok_or_else(|| DesignError::BudgetExceeded("scan size overflow".into()))?;
        if bytes > 2 * 1024 * 1024 || files.len() >= 256 {
            return Err(DesignError::BudgetExceeded(
                "static scan file/byte allowance".into(),
            ));
        }
        let content = std::fs::read(path)?;
        if content.len() as u64 != size {
            return Err(DesignError::StaleSource(
                "source changed during static scan".into(),
            ));
        }
        let hash = byte_hash(&content);
        let text = std::str::from_utf8(&content)
            .map_err(|_| DesignError::InvalidInput("design source is not UTF-8".into()))?;
        files.insert(relative.clone(), hash.clone());
        if extension == "json" {
            let package: serde_json::Value = serde_json::from_str(text)?;
            if let Some(deps) = package.get("dependencies").and_then(|d| d.as_object()) {
                for name in [
                    "react",
                    "next",
                    "vue",
                    "svelte",
                    "tailwindcss",
                    "@radix-ui/themes",
                    "@mui/material",
                ] {
                    if let Some(value) = deps
                        .get(name)
                        .and_then(|v| v.as_str())
                        .filter(|v| literal(v) && v.len() <= 64)
                    {
                        facts.push(DesignFact {
                            kind: "declared_dependency".into(),
                            name: name.into(),
                            value: value.into(),
                            path: relative.clone(),
                            source_hash: hash.clone(),
                            line: 1,
                            confidence: "declared; installation not verified".into(),
                        });
                    }
                }
            }
            continue;
        }
        for (index, line) in text.lines().enumerate() {
            if line.len() > 4096 {
                continue;
            }
            let line_number = u32::try_from(index + 1)
                .map_err(|_| DesignError::BudgetExceeded("source lines".into()))?;
            let pattern = if extension == "css" { &css } else { &theme };
            for capture in pattern.captures_iter(line) {
                let value = capture[2].trim();
                let name = &capture[1];
                if !literal(value)
                    || name.to_ascii_lowercase().contains("token") && !name.starts_with("--")
                {
                    continue;
                }
                if extension != "css"
                    && ![
                        "color",
                        "background",
                        "primary",
                        "secondary",
                        "spacing",
                        "font",
                        "radius",
                        "size",
                        "weight",
                    ]
                    .iter()
                    .any(|key| name.to_ascii_lowercase().contains(key))
                {
                    continue;
                }
                facts.push(DesignFact {
                    kind: if extension == "css" {
                        "css_token"
                    } else {
                        "static_theme_literal"
                    }
                    .into(),
                    name: name.into(),
                    value: value.into(),
                    path: relative.clone(),
                    source_hash: hash.clone(),
                    line: line_number,
                    confidence: "observed literal; cascade and execution unknown".into(),
                });
            }
            if let Some(capture) = props.captures(line) {
                facts.push(DesignFact {
                    kind: "component_props_declaration".into(),
                    name: capture[1].into(),
                    value: "declaration present; full type unresolved".into(),
                    path: relative.clone(),
                    source_hash: hash.clone(),
                    line: line_number,
                    confidence: "static declaration only".into(),
                });
            }
            if (line.contains("process.env")
                || line.contains("defineConfig(")
                || line.contains("...theme"))
                && unresolved.len() < 128
            {
                unresolved.push(format!(
                    "{relative}:{line_number}: computed configuration was not executed"
                ));
            }
            if facts.len() > 512 {
                return Err(DesignError::BudgetExceeded("static fact allowance".into()));
            }
        }
    }
    if facts.is_empty() {
        unresolved.push("No supported static design facts found".into());
    }
    let fingerprint = digest(&(relative_root, &files))?;
    Ok(DesignSystemSnapshot {
        root: relative_root.into(),
        fingerprint,
        files,
        facts,
        unresolved,
        excluded,
    })
}
pub fn validate_current(
    ctx: &AuthorizedDesignContext,
    snapshot: &DesignSystemSnapshot,
) -> DesignResult<()> {
    if extract_system_at(ctx, &snapshot.root)?.fingerprint != snapshot.fingerprint {
        return Err(DesignError::StaleSource(
            "design system source changed; sync again".into(),
        ));
    }
    Ok(())
}
