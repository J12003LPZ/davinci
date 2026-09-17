//! Conservative approval-risk helpers for `permission.rs` (no TypeScript counterpart).
//! This is not an OS sandbox: approved project checks still execute project code.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{
    has_windows_drive_prefix, is_symlink_escape, project_relative_with_boundary,
    FilesystemBoundaryPolicy,
};

#[derive(Debug)]
pub(super) struct FileTarget {
    pub subject: String,
    pub outside: bool,
    pub symlink_escape: bool,
    pub protected: bool,
    pub secret: bool,
    pub destructive: bool,
}

fn normalized_parts(path: &str) -> Vec<String> {
    path.replace('\\', "/")
        .split('/')
        // Windows ignores trailing dots/spaces and supports alternate streams.
        .map(|part| {
            part.split(':')
                .next()
                .unwrap_or(part)
                .trim_end_matches(['.', ' '])
                .to_ascii_lowercase()
        })
        .collect()
}

pub(super) fn is_secret_path(path: &str) -> bool {
    let parts = normalized_parts(path);
    if parts.iter().any(|part| {
        matches!(
            part.as_str(),
            ".ssh" | ".aws" | ".azure" | ".kube" | ".gnupg" | ".docker" | "secrets" | ".secrets"
        )
    }) {
        return true;
    }
    let Some(name) = parts.last() else {
        return false;
    };
    matches!(
        name.as_str(),
        ".env"
            | ".envrc"
            | "credentials"
            | "credentials.json"
            | ".credentials.json"
            | "secrets.json"
            | "auth.json"
            | ".netrc"
            | ".npmrc"
            | ".pypirc"
            | ".pgpass"
            | ".git-credentials"
            | ".dockercfg"
            | ".htpasswd"
            | "service-account.json"
            | "secring.gpg"
            | "settings.xml"
            | "gradle.properties"
    ) || name.starts_with(".env.")
        || name.starts_with("id_rsa")
        || name.starts_with("id_ed25519")
        || name.starts_with("id_ecdsa")
        || name.starts_with("id_dsa")
        || [
            ".pem",
            ".key",
            ".pfx",
            ".p12",
            ".p8",
            ".keystore",
            ".jks",
            ".env",
            ".kdbx",
        ]
        .iter()
        .any(|suffix| name.ends_with(suffix))
}

pub(super) fn is_protected_path(path: &str) -> bool {
    is_secret_path(path)
        || normalized_parts(path).iter().any(|part| {
            matches!(part.as_str(), ".pi" | ".davinci" | ".git")
                || matches!(
                    part.as_str(),
                    ".pi_patch_journal.json" | ".davinci_patch_journal.json"
                )
        })
}

fn resolved_path(cwd: &Path, raw: &str) -> PathBuf {
    let given = Path::new(raw);
    if given.is_absolute() || has_windows_drive_prefix(raw) {
        given.to_path_buf()
    } else {
        cwd.join(given)
    }
}

fn target(
    cwd: &Path,
    raw: &str,
    boundary: &FilesystemBoundaryPolicy,
    destructive: bool,
) -> FileTarget {
    let (subject, outside) = project_relative_with_boundary(cwd, raw, Some(boundary));
    let joined = resolved_path(cwd, raw);
    let root = boundary.root.as_deref().unwrap_or(cwd);
    let mut protected = is_protected_path(&subject);
    let mut secret = is_secret_path(&subject);
    // A harmless-looking symlink into an in-root credential/config directory
    // is still sensitive. Check existing ancestors for not-yet-created files.
    let mut existing = joined.as_path();
    loop {
        if let Ok(canonical) = existing.canonicalize() {
            let canonical = canonical.to_string_lossy();
            protected |= is_protected_path(&canonical);
            secret |= is_secret_path(&canonical);
            break;
        }
        let Some(parent) = existing.parent() else {
            break;
        };
        existing = parent;
    }
    FileTarget {
        subject,
        outside,
        symlink_escape: is_symlink_escape(root, &joined),
        protected,
        secret,
        destructive,
    }
}

/// Parse the same patch grammar the executor uses. Never match a compound
/// display string against a rule: each individual target needs authorization.
pub(super) fn file_targets(
    tool: &str,
    args: &Value,
    cwd: &Path,
    boundary: &FilesystemBoundaryPolicy,
) -> Result<Vec<FileTarget>, String> {
    if tool == "apply_patch" {
        let input = args
            .get("input")
            .and_then(Value::as_str)
            .ok_or_else(|| "apply_patch is missing its input".to_string())?;
        let parsed = crate::apply_patch::parse_codex_patch(input)?;
        if parsed.actions.is_empty() {
            return Err("apply_patch contains no file targets".into());
        }
        return Ok(parsed
            .actions
            .into_iter()
            .map(|action| {
                let (path, destructive) = match action {
                    crate::apply_patch::FileAction::Add { path, .. }
                    | crate::apply_patch::FileAction::Update { path, .. } => (path, false),
                    crate::apply_patch::FileAction::Delete { path } => (path, true),
                };
                target(cwd, &path, boundary, destructive)
            })
            .collect());
    }
    if matches!(
        tool,
        "read"
            | "grep"
            | "find"
            | "ls"
            | "write"
            | "edit"
            | "notebook_edit"
            | "lsp_definition"
            | "lsp_references"
            | "lsp_hover"
            | "lsp_document_symbols"
            | "lsp_workspace_symbols"
            | "lsp_implementations"
            | "lsp_type_definition"
            | "lsp_diagnostics"
    ) {
        if matches!(tool, "write" | "edit" | "notebook_edit")
            && args
                .get("path")
                .and_then(Value::as_str)
                .is_none_or(|path| path.trim().is_empty())
        {
            return Err(format!("{tool} is missing a nonempty path"));
        }
        let raw = args.get("path").and_then(Value::as_str).unwrap_or(".");
        return Ok(vec![target(cwd, raw, boundary, false)]);
    }
    Ok(Vec::new())
}

/// Recognize a deliberately small literal command language. Quotes, expansion,
/// response files, background jobs and interpreter injection require approval.
/// The shared shell analyzer remains read-only and supplies the command profiles.
pub(super) fn routine_local_shell(
    tool: &str,
    args: &Value,
    command: &str,
    cwd: &Path,
    boundary: &FilesystemBoundaryPolicy,
) -> bool {
    if tool == "write_stdin"
        || args.get("run_in_background").and_then(Value::as_bool) == Some(true)
        || args.get("background").and_then(Value::as_bool) == Some(true)
    {
        return false;
    }
    // Keep the policy root fixed when the command supplies a different cwd.
    let mut effective_boundary = boundary.clone();
    effective_boundary
        .root
        .get_or_insert_with(|| cwd.to_path_buf());
    let boundary = &effective_boundary;
    let mut execution_cwd = cwd.to_path_buf();
    let mut has_cwd_override = false;
    for key in ["cwd", "workdir", "work_dir", "directory"] {
        if let Some(value) = args.get(key) {
            let Some(raw) = value.as_str() else {
                return false;
            };
            if has_cwd_override {
                // Multiple aliases make the executor's choice ambiguous.
                return false;
            }
            let path = target(cwd, raw, boundary, false);
            if path.outside || path.protected {
                return false;
            }
            execution_cwd = resolved_path(cwd, raw);
            has_cwd_override = true;
        }
    }
    let literal = command.replace("2>&1", "");
    if literal.contains([
        '\'', '"', '\\', '$', '%', '`', '(', ')', '{', '}', '<', '>', '*', '?', '[', ']', '~', '@',
        '^', '#',
    ]) || literal.replace("&&", "").contains('&')
        || literal.trim_end().ends_with(['&', '|'])
        || literal.trim_start().starts_with(['&', '|', ';'])
    {
        return false;
    }
    // The shared splitter drops empty segments; approval must not silently
    // turn a malformed chain into a list of individually safe commands.
    let chain = literal
        .trim()
        .replace("&&", ";")
        .replace("||", ";")
        .replace(['|', '\n', '\r'], ";");
    if chain.split(';').any(|part| part.trim().is_empty()) {
        return false;
    }
    let (segments, malformed) = crate::shell_policy::split_shell_segments_with_diagnostic(&literal);
    if malformed || segments.is_empty() {
        return false;
    }
    segments.iter().all(|segment| {
        let words: Vec<_> = segment.split_whitespace().collect();
        let Some((program, rest)) = words.split_first() else {
            return false;
        };
        // Check every argument, including --option=path, for sensitive or
        // external targets. Colons can hide git revision paths or URL schemes.
        if words.iter().any(|word| {
            let value = word.split_once('=').map(|(_, value)| value).unwrap_or(word);
            value.contains(':') || value.is_empty() || {
                let path = target(&execution_cwd, value, boundary, false);
                path.outside || path.protected
            }
        }) {
            return false;
        }
        let program = program.to_ascii_lowercase();
        if program == "echo" {
            // Literal echo arguments do not become programs (`echo rm`).
            return true;
        }
        let recognized = match program.as_str() {
            "git" | "git.exe" => !rest.iter().any(|arg| {
                arg.starts_with("-C")
                    || arg.starts_with("--git-dir")
                    || arg.starts_with("--work-tree")
                    || arg.starts_with("--config")
                    || *arg == "-c"
            }),
            "cargo" => {
                matches!(
                    rest.first(),
                    Some(&"test" | &"check" | &"build" | &"clippy" | &"tree" | &"metadata")
                ) && rest
                    .iter()
                    .any(|arg| matches!(*arg, "--offline" | "--frozen"))
                    && !rest.iter().any(|arg| {
                        arg.starts_with("--config")
                            || arg.starts_with("--registry")
                            || arg.starts_with("--index")
                            || arg.starts_with("-Z")
                    })
                    || (rest.first() == Some(&"fmt") && rest.contains(&"--check"))
            }
            "cat" | "head" | "tail" | "grep" | "rg" | "find" | "fd" | "ls" | "dir" | "pwd"
            | "wc" | "sort" | "uniq" | "diff" | "stat" | "tree" | "which" | "where"
            | "where.exe" | "type" | "get-content" | "get-childitem" | "get-item"
            | "get-location" | "select-string" => true,
            _ => false,
        };
        recognized
            && matches!(
                crate::shell_policy::evaluate(
                    crate::shell_policy::ShellPolicyProfile::ReadAndTest,
                    segment
                ),
                crate::shell_policy::ShellCommandDecision::Allowed
            )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_names_cover_case_streams_and_nested_credentials() {
        for path in [
            ".DaVinci/settings.json",
            "nested/.pi/mcp.json",
            ".git./config",
            ".env:stream",
            "config/key.p12",
            "nested/.ssh/id_ed25519",
        ] {
            assert!(is_protected_path(path), "{path}");
        }
        assert!(!is_secret_path(".git/config"));
        assert!(!is_protected_path("src/environment.rs"));
        assert!(!is_protected_path(".pinned/file.txt"));
    }
}
