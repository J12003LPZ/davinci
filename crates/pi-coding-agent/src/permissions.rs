//! Where the permission policy comes from and where a granted rule goes.
//!
//! No TypeScript counterpart (see `pi-agent/src/permission.rs`). The user file
//! (`~/.pi/agent/settings.json`) and the project file (`.pi/settings.json`)
//! each carry a `permissions` block; the project's is read only when the
//! project is trusted, because a checkout that could grant itself the shell
//! by shipping a settings file would make trust meaningless.

use std::path::{Path, PathBuf};

use pi_agent::{PermissionMode, PermissionPolicy, PermissionRule};

use crate::settings::{
    load_settings, load_settings_file, with_settings_lock, PermissionSettings, CONFIG_DIR_NAME,
};

/// The two files a policy is assembled from, kept apart so `/permissions`
/// can say which rule came from where.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermissionSources {
    pub user: PermissionSettings,
    /// `None` when the project is not trusted: its rules are not in force.
    pub project: Option<PermissionSettings>,
}

impl PermissionSources {
    pub fn load(agent_dir: &Path, cwd: &Path, override_trust: Option<bool>) -> Self {
        let user_settings = load_settings(agent_dir);
        let trusted = crate::trust::resolve_project_trusted(
            agent_dir,
            cwd,
            override_trust,
            user_settings.default_project_trust.as_deref(),
            &user_settings.trusted_projects,
        );
        let project = trusted.then(|| {
            load_settings_file(&project_settings_path(cwd))
                .permissions
                .unwrap_or_default()
        });
        Self {
            user: user_settings.permissions.unwrap_or_default(),
            project,
        }
    }

    /// The mode in force before any flag: the project's word, then the
    /// user's, then `ask`.
    pub fn mode(&self) -> PermissionMode {
        self.project
            .as_ref()
            .and_then(|project| project.mode.as_deref())
            .or(self.user.mode.as_deref())
            .and_then(PermissionMode::parse)
            .unwrap_or_default()
    }

    /// Assemble the policy. `flag_mode` is `--permission-mode` / `--sandbox`
    /// (or `PI_PERMISSION_MODE` when neither was given) and wins outright.
    pub fn policy(&self, flag_mode: Option<PermissionMode>) -> PermissionPolicy {
        let mode = flag_mode
            .or_else(|| {
                std::env::var("PI_PERMISSION_MODE")
                    .ok()
                    .and_then(|value| PermissionMode::parse(&value))
            })
            .unwrap_or_else(|| self.mode());
        let rules = |pick: fn(&PermissionSettings) -> &Vec<String>| -> Vec<PermissionRule> {
            pick(&self.user)
                .iter()
                .chain(self.project.iter().flat_map(|project| pick(project).iter()))
                .filter_map(|text| PermissionRule::parse(text))
                .collect()
        };
        PermissionPolicy {
            mode,
            allow: rules(|settings| &settings.allow),
            deny: rules(|settings| &settings.deny),
            session_allow: Vec::new(),
        }
    }
}

pub fn policy_for(
    agent_dir: &Path,
    cwd: &Path,
    override_trust: Option<bool>,
    flag_mode: Option<PermissionMode>,
) -> PermissionPolicy {
    PermissionSources::load(agent_dir, cwd, override_trust).policy(flag_mode)
}

pub fn project_settings_path(cwd: &Path) -> PathBuf {
    cwd.join(CONFIG_DIR_NAME).join("settings.json")
}

/// "Always allow in this project": append the rule to `.pi/settings.json`
/// under `permissions.allow`. The file is edited as JSON rather than through
/// `Settings`, so a key this build does not model survives untouched, and it
/// is created with only the `permissions` block when it does not exist.
pub fn remember_project_rule(cwd: &Path, rule: &str) -> Result<PathBuf, String> {
    let path = project_settings_path(cwd);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    with_settings_lock(&path, || {
        let mut value = match std::fs::read_to_string(&path) {
            Ok(raw) if !raw.trim().is_empty() => serde_json::from_str(&raw)
                .map_err(|err| format!("{} is not valid JSON: {err}", path.display()))?,
            _ => serde_json::Value::Object(Default::default()),
        };
        let object = value
            .as_object_mut()
            .ok_or_else(|| format!("{} is not a JSON object", path.display()))?;
        let permissions = object
            .entry("permissions")
            .or_insert_with(|| serde_json::Value::Object(Default::default()));
        let permissions = permissions
            .as_object_mut()
            .ok_or_else(|| "permissions is not an object".to_string())?;
        let allow = permissions
            .entry("allow")
            .or_insert_with(|| serde_json::Value::Array(Vec::new()));
        let allow = allow
            .as_array_mut()
            .ok_or_else(|| "permissions.allow is not a list".to_string())?;
        if !allow.iter().any(|item| item.as_str() == Some(rule)) {
            allow.push(serde_json::Value::String(rule.to_string()));
        }
        let encoded = serde_json::to_string_pretty(&value).map_err(|err| err.to_string())?;
        std::fs::write(&path, format!("{encoded}\n")).map_err(|err| err.to_string())?;
        Ok(path.clone())
    })
}

/// The rows `/permissions` says: the mode, then every rule by source.
pub fn describe(sources: &PermissionSources, policy: &PermissionPolicy) -> Vec<String> {
    let mut rows = vec![format!(
        "permission mode {} · {}",
        policy.mode.as_str(),
        policy.mode.describe()
    )];
    let list = |label: &str, rules: &[String]| -> Option<String> {
        (!rules.is_empty()).then(|| format!("{label} · {}", rules.join(", ")))
    };
    rows.extend(list("allow (user)", &sources.user.allow));
    rows.extend(list("deny (user)", &sources.user.deny));
    match &sources.project {
        Some(project) => {
            rows.extend(list("allow (project)", &project.allow));
            rows.extend(list("deny (project)", &project.deny));
        }
        None => rows.push("project rules ignored · the project is not trusted (/trust)".into()),
    }
    let session: Vec<String> = policy
        .session_allow
        .iter()
        .map(ToString::to_string)
        .collect();
    rows.extend(list("allow (this session)", &session));
    if rows.len() == 1 {
        rows.push("no rules · /permissions <read-only|ask|edits|auto> sets the mode".into());
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{save_settings, Settings};

    fn user_dir_with(permissions: PermissionSettings, trusted: &[String]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings {
            permissions: Some(permissions),
            trusted_projects: trusted.to_vec(),
            ..Settings::default()
        };
        save_settings(dir.path(), &settings).unwrap();
        dir
    }

    fn block(mode: Option<&str>, allow: &[&str], deny: &[&str]) -> PermissionSettings {
        PermissionSettings {
            mode: mode.map(str::to_string),
            allow: allow.iter().map(|s| s.to_string()).collect(),
            deny: deny.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn user_and_trusted_project_rules_are_unioned_and_the_project_mode_wins() {
        let project = tempfile::tempdir().unwrap();
        let agent_dir = user_dir_with(block(Some("edits"), &["bash(git *)"], &["bash(rm *)"]), &[]);
        std::fs::create_dir_all(project.path().join(".pi")).unwrap();
        std::fs::write(
            project_settings_path(project.path()),
            r#"{"permissions": {"mode": "ask", "allow": ["bash(cargo *)"]}, "other": 1}"#,
        )
        .unwrap();

        let sources = PermissionSources::load(agent_dir.path(), project.path(), Some(true));
        assert!(sources.project.is_some());
        let policy = sources.policy(None);
        assert_eq!(policy.mode, PermissionMode::Ask);
        let allow: Vec<String> = policy.allow.iter().map(ToString::to_string).collect();
        assert_eq!(allow, ["bash(git *)", "bash(cargo *)"]);
        assert_eq!(policy.deny.len(), 1);

        // A flag beats both files.
        assert_eq!(
            sources.policy(Some(PermissionMode::Auto)).mode,
            PermissionMode::Auto
        );
    }

    #[test]
    fn an_untrusted_project_grants_nothing() {
        let project = tempfile::tempdir().unwrap();
        let agent_dir = user_dir_with(block(None, &["edit"], &[]), &[]);
        std::fs::create_dir_all(project.path().join(".pi")).unwrap();
        std::fs::write(
            project_settings_path(project.path()),
            r#"{"permissions": {"mode": "auto", "allow": ["bash"]}}"#,
        )
        .unwrap();

        let sources = PermissionSources::load(agent_dir.path(), project.path(), Some(false));
        assert_eq!(sources.project, None);
        let policy = sources.policy(None);
        assert_eq!(
            policy.mode,
            PermissionMode::Ask,
            "the user file set no mode"
        );
        assert_eq!(
            policy
                .allow
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["edit"]
        );
        let rows = describe(&sources, &policy);
        assert!(
            rows.iter().any(|row| row.contains("not trusted")),
            "{rows:?}"
        );
    }

    #[test]
    fn remembering_a_project_rule_creates_the_file_and_keeps_other_keys() {
        let project = tempfile::tempdir().unwrap();
        let path = remember_project_rule(project.path(), "bash(git status *)").unwrap();
        assert_eq!(path, project_settings_path(project.path()));
        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            written,
            serde_json::json!({"permissions": {"allow": ["bash(git status *)"]}})
        );

        std::fs::write(
            &path,
            r#"{"theme": "dark", "permissions": {"deny": ["bash(rm *)"], "allow": ["edit"]}}"#,
        )
        .unwrap();
        remember_project_rule(project.path(), "write").unwrap();
        remember_project_rule(project.path(), "write").unwrap();
        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written["theme"], "dark");
        assert_eq!(
            written["permissions"]["deny"],
            serde_json::json!(["bash(rm *)"])
        );
        assert_eq!(
            written["permissions"]["allow"],
            serde_json::json!(["edit", "write"])
        );
    }

    #[test]
    fn describe_lists_the_mode_and_every_source() {
        let sources = PermissionSources {
            user: block(Some("ask"), &["read"], &[]),
            project: Some(block(None, &["bash(cargo *)"], &["bash(rm *)"])),
        };
        let mut policy = sources.policy(None);
        policy.remember("write");
        let rows = describe(&sources, &policy);
        assert_eq!(
            rows,
            [
                "permission mode ask · read tools run; edits and shell commands ask",
                "allow (user) · read",
                "allow (project) · bash(cargo *)",
                "deny (project) · bash(rm *)",
                "allow (this session) · write",
            ]
        );
    }
}
