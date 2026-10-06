use super::*;
use serde_json::json;

#[test]
fn audit_url_authorization_uses_the_transport_hostname() {
    for (raw, host) in [
        (
            r"https://blocked.example\@allowed.example/",
            "blocked.example",
        ),
        ("https://%62locked.example/path", "blocked.example"),
        (
            "https://allowed.example@blocked.example/",
            "blocked.example",
        ),
        ("https://[2001:db8::1]:8443/path", "[2001:db8::1]"),
        (" HTTPS://BLOCKED.EXAMPLE/path ", "blocked.example"),
    ] {
        assert_eq!(host_of(raw), host, "{raw}");
        let mut p = policy(PermissionMode::Ask);
        p.allow
            .push(PermissionRule::parse("web_fetch(allowed.example)").unwrap());
        p.deny
            .push(PermissionRule::parse(&format!("web_fetch({host})")).unwrap());
        assert!(
            is_deny(&verdict(&p, "web_fetch", json!({"url":raw}))),
            "{raw}"
        );
    }
}

fn cwd() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from("C:\\work\\proj")
    } else {
        PathBuf::from("/work/proj")
    }
}

#[test]
fn plan_evidence_obeys_explicit_read_denies_even_with_blanket_allow() {
    for mode in PermissionMode::ALL {
        let mut p = policy(mode);
        p.allow.push(PermissionRule::bare("*"));
        p.deny
            .push(PermissionRule::parse("read(private/**)").unwrap());
        assert!(
            is_deny(&verdict(
                &p,
                "propose_plan",
                json!({
                    "expected_revision":0,
                    "evidence":[{"path":"private/notes.txt","finding":"Not authorized"}]
                })
            )),
            "{mode:?}"
        );
    }
}

#[test]
fn transaction_preimages_require_read_authority_even_with_edit_allow() {
    for (tool, args) in [
        ("write", json!({"path":".env", "content":"fixture"})),
        ("edit", json!({"path":".env", "oldText":"a", "newText":"b"})),
        ("notebook_edit", json!({"path":".env"})),
        (
            "apply_patch",
            json!({"input":"*** Begin Patch\n*** Delete File: .env\n*** End Patch"}),
        ),
        (
            "patch_preview",
            json!({"input":"*** Begin Patch\n*** Delete File: .env\n*** End Patch"}),
        ),
        ("patch_apply", json!({"id":"fixture", "paths":[".env"]})),
        ("patch_status", json!({"id":"fixture", "paths":[".env"]})),
        ("patch_rollback", json!({"id":"fixture", "paths":[".env"]})),
    ] {
        let mut p = policy(PermissionMode::Edits);
        p.allow.push(PermissionRule::bare(tool));
        let PermissionVerdict::Ask(request) = verdict(&p, tool, args.clone()) else {
            panic!("{tool} bypassed the preimage read approval");
        };
        assert_eq!(request.tool, tool);
        assert_eq!(request.args, args);
        assert!(request.session_rule.is_empty());
        p.allow.push(PermissionRule::bare("read"));
        assert!(
            matches!(verdict(&p, tool, args.clone()), PermissionVerdict::Allow),
            "{tool}"
        );
        p.deny.push(PermissionRule::bare("read"));
        assert!(is_deny(&verdict(&p, tool, args)), "{tool}");
    }
}

#[test]
fn transaction_git_observation_respects_nested_metadata_read_denies() {
    for rule in [
        "read(.git/config)",
        "read(.git/objects/**)",
        "read(private/**)",
    ] {
        let mut p = policy(PermissionMode::AlwaysApprove);
        p.deny.push(PermissionRule::parse(rule).unwrap());
        let args = json!({"id":"fixture", "paths":["a.txt"], "observe_commit":true});
        assert!(is_deny(&verdict(&p, "patch_status", args)), "{rule}");
        assert!(matches!(
            verdict(
                &p,
                "patch_status",
                json!({"id":"fixture", "paths":["a.txt"]})
            ),
            PermissionVerdict::Allow
        ));
    }
}

#[test]
fn five_named_modes_are_available_in_the_requested_order() {
    let names = [
        "Manual",
        "Accept Edits",
        "Plan Mode",
        "Auto Mode",
        "Always Approve",
    ];
    assert_eq!(PermissionMode::ALL.len(), names.len());
    for (mode, name) in PermissionMode::ALL.iter().zip(names) {
        assert_eq!(PermissionMode::parse(name), Some(*mode), "{name}");
    }
}

#[test]
fn plan_permission_cannot_be_overridden_by_saved_allow_rules() {
    let mut p = policy(PermissionMode::ReadOnly);
    p.allow.push(PermissionRule::bare("*"));
    for (tool, args) in [
        ("write", json!({"path":"src/main.rs", "content":"changed"})),
        ("bash", json!({"command":"rm -rf src"})),
        ("job_kill", json!({"job_id":"1"})),
        (
            "write_stdin",
            json!({"session_id":"1", "chars":"rm -rf src\n"}),
        ),
    ] {
        assert!(
            is_deny(&verdict(&p, tool, args)),
            "Plan Mode allowed {tool}"
        );
    }
}

#[test]
fn auto_escalates_external_destructive_and_sensitive_actions() {
    let p = policy(PermissionMode::Auto);
    for (tool, args) in [
        ("write", json!({"path":"../outside.txt"})),
        ("write", json!({"path":".davinci/settings.json"})),
        ("read", json!({"path":".env"})),
        ("bash", json!({"command":"git push origin main"})),
        ("bash", json!({"command":"rm -rf src"})),
        ("bash", json!({"command":"curl https://example.com"})),
        (
            "bash",
            json!({"command":"python -c 'import os; os.remove(\"a\")'"}),
        ),
        ("unknown_tool", json!({})),
    ] {
        assert!(
            is_ask(&verdict(&p, tool, args.clone())),
            "Auto allowed {tool}: {args}"
        );
    }
    assert_eq!(
        verdict(&p, "write", json!({"path":"src/main.rs"})),
        PermissionVerdict::Allow
    );
    assert!(is_ask(&verdict(
        &p,
        "bash",
        json!({"command":"cargo test --offline"})
    )));
    let mut isolated = p.clone();
    isolated.execution_isolated = true;
    assert_eq!(
        verdict(&isolated, "bash", json!({"command":"cargo test --offline"})),
        PermissionVerdict::Allow
    );
}

#[test]
fn auto_without_an_os_sandbox_asks_before_running_workspace_code() {
    // Auto may edit build.rs or a test and then run it. Without isolation
    // that chain is arbitrary code execution, so every code-running check asks.
    let p = policy(PermissionMode::Auto);
    let mut isolated = p.clone();
    isolated.execution_isolated = true;
    for command in ["cargo build --offline", "cargo clippy --offline"] {
        assert!(
            is_ask(&verdict(&p, "bash", json!({"command":command}))),
            "unconfined Auto allowed {command}"
        );
    }
    for command in [
        "cargo test --offline",
        "cargo check --offline",
        "git status && cargo test --offline",
    ] {
        assert!(
            is_ask(&verdict(&p, "bash", json!({"command":command}))),
            "unconfined Auto allowed {command}"
        );
        assert_eq!(
            verdict(&isolated, "bash", json!({"command":command})),
            PermissionVerdict::Allow,
            "isolated Auto asked for {command}"
        );
    }
    for command in [
        "cargo fmt --check",
        "cargo tree --offline",
        "cargo metadata --offline",
        "git status",
    ] {
        assert_eq!(
            verdict(&p, "bash", json!({"command":command})),
            PermissionVerdict::Allow,
            "{command}"
        );
    }
    assert_eq!(
        verdict(&p, "write", json!({"path":"build.rs"})),
        PermissionVerdict::Allow
    );
}

#[test]
fn multi_file_patch_cannot_hide_a_sensitive_target() {
    let p = policy(PermissionMode::Edits);
    let patch = "*** Begin Patch\n*** Add File: src/ok.rs\n+ok\n*** Add File: .davinci/settings.json\n+{}\n*** End Patch";
    assert!(is_ask(&verdict(&p, "apply_patch", json!({"input":patch}))));
}

#[test]
fn five_mode_names_and_order_are_public_contract() {
    let names: Vec<_> = PermissionMode::ALL
        .iter()
        .map(|mode| mode.as_str())
        .collect();
    assert_eq!(
        names,
        ["ask", "edits", "read-only", "auto", "always-approve"]
    );
    for name in names {
        assert_eq!(PermissionMode::parse(name).unwrap().as_str(), name);
    }
}

#[test]
fn auto_mode_escalates_unknown_destructive_and_external_actions() {
    let p = policy(PermissionMode::Auto);
    for command in [
        "rm -rf build",
        "git push origin main",
        "curl https://example.com/upload",
        "unknown-tool",
        "git status && rm x",
    ] {
        assert!(
            is_ask(&verdict(&p, "bash", json!({"command": command}))),
            "{command}"
        );
    }
    assert!(is_ask(&verdict(
        &p,
        "write",
        json!({"path": "../outside.txt"})
    )));
    assert!(is_ask(&verdict(&p, "custom_tool", json!({}))));
    assert_eq!(
        verdict(&p, "bash", json!({"command": "git status"})),
        PermissionVerdict::Allow
    );
    assert_eq!(
        verdict(&p, "write", json!({"path": "src/lib.rs"})),
        PermissionVerdict::Allow
    );
}

#[test]
fn plan_mode_cannot_be_overridden_by_allow_rules() {
    let mut p = policy(PermissionMode::ReadOnly);
    p.allow.push(PermissionRule::bare("*"));
    for (tool, args) in [
        ("write", json!({"path": "x.rs"})),
        ("bash", json!({"command": "rm x"})),
        ("job_kill", json!({})),
        ("agent_message", json!({})),
    ] {
        assert!(is_deny(&verdict(&p, tool, args)), "{tool}");
    }
}

#[test]
fn accept_edits_requires_approval_for_davinci_git_and_secret_paths() {
    let p = policy(PermissionMode::Edits);
    for path in [
        ".davinci/settings.json",
        ".git/config",
        ".env",
        "config/credentials.json",
    ] {
        assert!(
            is_ask(&verdict(&p, "write", json!({"path": path}))),
            "{path}"
        );
    }
    let patch = "*** Begin Patch\n*** Add File: src/a.rs\n+ok\n*** Add File: .davinci/settings.json\n+{}\n*** End Patch";
    assert!(is_ask(&verdict(&p, "apply_patch", json!({"input": patch}))));
}

#[test]
fn mode_labels_cycle_and_legacy_ids_remain_compatible() {
    let ids = ["ask", "edits", "read-only", "auto", "always-approve"];
    for (index, mode) in PermissionMode::ALL.into_iter().enumerate() {
        assert_eq!(mode.as_str(), ids[index]);
        assert_eq!(PermissionMode::parse(mode.label()), Some(mode));
        assert_eq!(mode.next(), PermissionMode::ALL[(index + 1) % ids.len()]);
    }
    for alias in [
        "full-access",
        "danger-full-access",
        "yolo",
        "bypass",
        "autopilot",
    ] {
        assert_eq!(
            PermissionMode::parse(alias),
            Some(PermissionMode::AlwaysApprove)
        );
    }
    assert_eq!(
        PermissionPolicy::default().mode,
        PermissionMode::AlwaysApprove
    );
    assert_eq!(PermissionMode::default(), PermissionMode::Ask);
}

#[test]
fn every_mode_honors_explicit_denies_and_hard_read_boundaries() {
    for mode in PermissionMode::ALL {
        let mut p = policy(mode);
        p.allow.push(PermissionRule::bare("*"));
        p.remember("*");
        p.filesystem_boundary.allow_git_metadata = true;
        p.filesystem_boundary.root = Some(cwd());
        p.deny.push(PermissionRule::parse("write(.git/*)").unwrap());
        assert!(
            is_deny(&verdict(&p, "write", json!({"path":".git/config"}))),
            "{mode:?}"
        );
        p.set_read_outside_root_policy(ReadOutsideRootPolicy::Deny);
        assert!(
            is_deny(&verdict(&p, "read", json!({"path":"../outside.txt"}))),
            "{mode:?}"
        );
    }
}

#[test]
fn plan_freeze_precedes_session_grants_and_side_effect_class_hints() {
    let mut p = policy(PermissionMode::ReadOnly);
    p.remember("*");
    // MCP hints cannot reclassify a built-in mutation as a safe read.
    p.mcp_read_only.insert("job_kill".into());
    for tool in [
        "job_kill",
        "agent_message",
        "agent_stop",
        "task_create",
        "task_update",
        "graph_submit",
        "unknown_tool",
    ] {
        assert!(is_deny(&verdict(&p, tool, json!({}))), "{tool}");
    }
    for tool in [
        "read",
        "grep",
        "find",
        "ls",
        "todo",
        "update_plan",
        "job_output",
        "task_list",
        "task_get",
    ] {
        assert_eq!(
            verdict(&p, tool, json!({})),
            PermissionVerdict::Allow,
            "{tool}"
        );
    }
}

#[test]
fn patch_rules_and_risk_checks_inspect_each_target() {
    let patch = "*** Begin Patch\n*** Add File: src/good.rs\n+ok\n*** Add File: private/key.pem\n+not-a-real-key\n*** End Patch";
    let args = json!({"input":patch});
    for mode in [
        PermissionMode::Ask,
        PermissionMode::Edits,
        PermissionMode::Auto,
    ] {
        let mut p = policy(mode);
        p.allow
            .push(PermissionRule::parse("apply_patch(src/*)").unwrap());
        assert!(
            is_ask(&verdict(&p, "apply_patch", args.clone())),
            "{mode:?}"
        );
        p.deny
            .push(PermissionRule::parse("apply_patch(private/*)").unwrap());
        assert!(
            is_deny(&verdict(&p, "apply_patch", args.clone())),
            "{mode:?}"
        );
    }
    for path in [
        ".pi/settings.json",
        ".davinci/settings.json",
        ".git/config",
        ".env.local",
        "../outside.txt",
    ] {
        for action in [
            format!("*** Add File: {path}\n+x"),
            format!("*** Delete File: {path}"),
            format!("*** Update File: {path}\n@@\n-old\n+new"),
        ] {
            let input =
                format!("*** Begin Patch\n*** Add File: src/good.rs\n+ok\n{action}\n*** End Patch");
            assert!(
                is_ask(&verdict(
                    &policy(PermissionMode::Edits),
                    "apply_patch",
                    json!({"input":input})
                )),
                "{path}"
            );
        }
    }
}

#[test]
fn auto_shell_matrix_escalates_unknown_syntax_network_and_path_changes() {
    let mut p = policy(PermissionMode::Auto);
    p.execution_isolated = true;
    for command in [
        "git status && cargo test --offline",
        "cargo check --offline",
        "git diff --stat",
        "pwd",
    ] {
        assert_eq!(
            verdict(&p, "bash", json!({"command":command})),
            PermissionVerdict::Allow,
            "{command}"
        );
    }
    for command in [
        "git status && curl https://example.com",
        "git status; rm x",
        "git status $(whoami)",
        "git log `whoami`",
        "diff <(cat a) b",
        "echo ok > out.txt",
        "echo 'unterminated",
        "git status &",
        "git -C ../elsewhere status",
        "cat ../outside.txt",
        "cat /etc/passwd",
        "cat .env",
        "cat config/credentials.json",
        "git show HEAD:.env",
        "cargo test --offline --manifest-path ../Cargo.toml",
        "cargo test --offline --target-dir ../build",
        "cargo test",
        "cargo build",
        "npx jest",
        "npm view package",
        "python -c 'print(1)'",
        "node --test --require=helper.js",
        "find . -delete",
        "git branch new-branch",
    ] {
        assert!(
            is_ask(&verdict(&p, "bash", json!({"command":command}))),
            "{command}"
        );
    }
    assert!(is_ask(&verdict(&p, "exec_command", json!({"cmd":"rm x"}))));
    assert!(is_ask(&verdict(
        &p,
        "bash",
        json!({"command":"git status", "cwd":"../other"})
    )));
    assert!(is_ask(&verdict(
        &p,
        "write_stdin",
        json!({"command":"git status"})
    )));
}

#[test]
fn malformed_shell_chains_are_not_routine_commands() {
    for command in [
        "git status && && git diff",
        "git status || | git diff",
        "git status;;git diff",
    ] {
        assert!(
            is_ask(&verdict(
                &policy(PermissionMode::Auto),
                "bash",
                json!({"command":command})
            )),
            "{command}"
        );
    }
}

#[test]
fn patch_symlinks_and_isolated_boundaries_apply_in_every_mode() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    for mode in PermissionMode::ALL {
        let mut p = policy(mode);
        p.filesystem_boundary.root = Some(root.clone());
        p.filesystem_boundary.enforce_root_for_mutations = true;
        p.allow.push(PermissionRule::bare("*"));
        let patch = "*** Begin Patch\n*** Add File: good.rs\n+ok\n*** Add File: ../outside/bad.rs\n+bad\n*** End Patch";
        assert!(
            is_deny(&p.decide("test", "apply_patch", &json!({"input":patch}), &root)),
            "{mode:?}"
        );
    }
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(&outside, root.join("link")).is_ok();
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_dir(&outside, root.join("link")).is_ok();
    if linked {
        let patch = "*** Begin Patch\n*** Add File: good.rs\n+ok\n*** Add File: link/new.rs\n+bad\n*** End Patch";
        let p = policy(PermissionMode::AlwaysApprove);
        assert!(is_deny(&p.decide(
            "test",
            "apply_patch",
            &json!({"input":patch}),
            &root
        )));
    }
}

#[test]
fn relative_targets_are_resolved_from_execution_cwd_not_policy_root() {
    let root = cwd();
    let execution_cwd = root.join("nested");
    let mut p = policy(PermissionMode::Edits);
    p.filesystem_boundary.root = Some(root.clone());
    p.deny
        .push(PermissionRule::parse("write(nested/blocked.rs)").unwrap());
    assert!(is_deny(&p.decide(
        "test",
        "write",
        &json!({"path":"blocked.rs"}),
        &execution_cwd
    )));
    assert_eq!(
        project_relative_with_boundary(&execution_cwd, "../ok.rs", Some(&p.filesystem_boundary)),
        ("ok.rs".into(), false)
    );
}

#[test]
fn boundary_alias_resolution_preserves_target_symlinks() {
    let dirs = ExtraRoots::new();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&dirs.extra, dirs.workspace.join("link")).unwrap();
    #[cfg(windows)]
    {
        let output = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(dirs.workspace.join("link"))
            .arg(&dirs.extra)
            .output()
            .unwrap();
        assert!(output.status.success());
    }
    let target = dirs.workspace.join("link/new/deep/file.rs");
    assert_eq!(
        boundary_relative_path(&dirs.workspace, &target),
        Some(PathBuf::from("link/new/deep/file.rs"))
    );
    assert_eq!(check_path_boundary(&dirs.workspace, &target), (false, true));
    for mode in PermissionMode::ALL {
        let mut p = dirs.policy(mode);
        p.allow.push(PermissionRule::bare("*"));
        assert!(is_deny(&dirs.write(&p, &target)), "{mode:?}");
        assert_eq!(
            p.filesystem_boundary
                .boundary_root_for(&dirs.workspace, &target),
            dirs.workspace
        );
    }
}

#[cfg(windows)]
#[test]
fn boundary_identity_strips_verbatim_prefix_before_parent_traversal() {
    let root = PathBuf::from(r"\\?\C:\work\proj");
    let target = root.join(r"nested\..\new\file.rs");
    assert_eq!(
        boundary_path_identity(&target),
        PathBuf::from("C:/work/proj/new/file.rs")
    );
    assert_eq!(check_path_boundary(&root, &target), (false, false));
    assert_eq!(
        project_relative(&root, "C:/work/proj/new/file.rs"),
        ("new/file.rs".into(), false)
    );
}

#[cfg(windows)]
#[test]
fn windows_short_root_alias_keeps_relative_rules_and_extra_grants() {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    #[link(name = "kernel32")]
    extern "system" {
        fn GetShortPathNameW(long: *const u16, short: *mut u16, size: u32) -> u32;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("LongWorkspaceDirectory");
    std::fs::create_dir(&root).unwrap();
    let long: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut short = vec![0u16; 32768];
    // SAFETY: the input is terminated and both buffers live for the call.
    let len = unsafe { GetShortPathNameW(long.as_ptr(), short.as_mut_ptr(), short.len() as u32) };
    assert!(len > 0 && (len as usize) < short.len());
    let alias = PathBuf::from(std::ffi::OsString::from_wide(&short[..len as usize]));
    let target = alias.join("new.rs");
    assert_eq!(check_path_boundary(&root, &target), (false, false));
    let mut p = policy(PermissionMode::Edits);
    p.filesystem_boundary.root = Some(root.clone());
    p.deny.push(PermissionRule::parse("write(new.rs)").unwrap());
    assert!(is_deny(&p.decide(
        "test",
        "write",
        &json!({"path":target}),
        &root
    )));
    let boundary = FilesystemBoundaryPolicy {
        extra_roots: vec![root.clone()],
        ..Default::default()
    };
    assert_eq!(
        boundary.extra_root_containing(&target),
        Some(root.as_path())
    );
}

#[test]
fn shell_workdir_changes_cannot_hide_an_outside_operand() {
    let root = cwd();
    let execution_cwd = root.join("nested");
    let mut p = policy(PermissionMode::Auto);
    p.filesystem_boundary.root = Some(root);
    for key in ["cwd", "workdir", "work_dir", "directory"] {
        let mut args = json!({"command":"cat ../outside.txt"});
        args[key] = json!("..");
        assert!(
            is_ask(&p.decide("test", "bash", &args, &execution_cwd)),
            "{key}"
        );
    }
}

#[test]
fn explicit_shell_denies_survive_substitution_and_always_approve() {
    let mut p = policy(PermissionMode::AlwaysApprove);
    p.deny.push(PermissionRule::parse("bash(rm *)").unwrap());
    for command in ["echo ok && rm file", "echo $(rm file)", "echo `rm file`"] {
        assert!(
            is_deny(&verdict(&p, "bash", json!({"command":command}))),
            "{command}"
        );
    }
}

#[test]
fn malformed_patches_never_gain_automatic_edit_permission() {
    for mode in [PermissionMode::Edits, PermissionMode::Auto] {
        for args in [
            json!({}),
            json!({"input":"not a patch"}),
            json!({"input":"*** Begin Patch\n*** End Patch"}),
        ] {
            assert!(
                is_deny(&verdict(&policy(mode), "apply_patch", args)),
                "{mode:?}"
            );
        }
    }
}

#[test]
fn rules_parse_bare_tools_and_patterns() {
    assert_eq!(
        PermissionRule::parse("bash"),
        Some(PermissionRule {
            tool: "bash".into(),
            pattern: None,
            specifier: None,
        })
    );
    assert_eq!(
        PermissionRule::parse(" bash(git *) "),
        Some(PermissionRule {
            tool: "bash".into(),
            pattern: Some("git *".into()),
            specifier: Some(RuleSpecifier::Subject("git *".into())),
        })
    );
    assert_eq!(PermissionRule::parse("bash()").unwrap().pattern, None);
    assert_eq!(PermissionRule::parse(""), None);
    assert_eq!(PermissionRule::parse("bash(git"), None);
    assert_eq!(PermissionRule::parse("two words"), None);
    assert_eq!(
        PermissionRule::parse("bash(git *)").unwrap().to_string(),
        "bash(git *)"
    );
}

#[test]
fn globs_match_runs_single_characters_and_slashes() {
    assert!(glob_matches("git *", "git status"));
    assert!(glob_matches("*", "anything at all"));
    assert!(glob_matches("src/**", "src/a/b/c.rs"));
    assert!(glob_matches("src/*.rs", "src/lib.rs"));
    assert!(glob_matches("?at", "cat"));
    assert!(!glob_matches("?at", "at"));
    assert!(!glob_matches("git *", "gitk"));
    assert!(!glob_matches("src/*.rs", "src/lib.ts"));
    assert!(glob_matches("a*b*c", "aXXbYYc"));
    assert!(!glob_matches("a*b*c", "aXXbYY"));
    assert!(glob_matches("", ""));
    assert!(!glob_matches("", "x"));
}

#[test]
fn a_trailing_star_rule_also_means_the_bare_prefix() {
    let rule = PermissionRule::parse("bash(git status *)").unwrap();
    assert!(rule.matches("bash", "git status"));
    assert!(rule.matches("bash", "git status --short"));
    assert!(!rule.matches("bash", "git stash"));
    assert!(!rule.matches("powershell", "git status"));
}

#[test]
fn a_bare_rule_covers_every_call_of_the_tool() {
    let rule = PermissionRule::parse("write").unwrap();
    assert!(rule.matches("write", "src/lib.rs"));
    assert!(rule.matches("write", ""));
    assert!(!rule.matches("edit", "src/lib.rs"));
}

#[test]
fn a_pattern_rule_never_matches_a_tool_without_a_subject() {
    let rule = PermissionRule::parse("vector_search(*)").unwrap();
    assert!(!rule.matches("vector_search", ""));
}

#[test]
fn subjects_are_commands_or_project_relative_paths() {
    let cwd = cwd();
    assert_eq!(
        subject_of("bash", &json!({"command": "  git status \n"}), &cwd),
        ("git status".into(), false)
    );
    assert_eq!(
        subject_of("write", &json!({"path": "src\\lib.rs"}), &cwd),
        ("src/lib.rs".into(), false)
    );
    assert_eq!(
        subject_of(
            "read",
            &json!({"path": cwd.join("a").join("b.txt").to_string_lossy()}),
            &cwd
        ),
        ("a/b.txt".into(), false)
    );
    assert_eq!(
        subject_of("edit", &json!({"path": "src/../../secret"}), &cwd),
        (
            if cfg!(windows) {
                "C:/work/secret".to_string()
            } else {
                "/work/secret".to_string()
            },
            true
        )
    );
    assert_eq!(subject_of("ls", &json!({}), &cwd), (".".into(), false));
    assert_eq!(
        subject_of("vector_search", &json!({"q": "x"}), &cwd),
        (String::new(), false)
    );
}

#[test]
fn session_rules_name_the_program_and_its_subcommand() {
    let rule = |command: &str| session_rule_for("bash", command).to_string();
    assert_eq!(rule("git status --short"), "bash(git status *)");
    assert_eq!(rule("cargo test -p davinci-agent"), "bash(cargo test *)");
    assert_eq!(rule("rm -rf build"), "bash(rm *)");
    assert_eq!(rule("./run.sh now"), "bash(./run.sh *)");
    assert_eq!(rule("python script.py"), "bash(python *)");
    assert_eq!(rule("ls && rm x"), "bash(ls *)");
    assert_eq!(rule("make"), "bash(make *)");
    assert_eq!(session_rule_for("write", "src/lib.rs").to_string(), "write");
    assert_eq!(
        session_rule_for("vector_search", "").to_string(),
        "vector_search"
    );
}

fn policy(mode: PermissionMode) -> PermissionPolicy {
    PermissionPolicy::new(mode)
}

fn verdict(policy: &PermissionPolicy, tool: &str, args: Value) -> PermissionVerdict {
    policy.decide("call_1", tool, &args, &cwd())
}

fn is_ask(verdict: &PermissionVerdict) -> bool {
    matches!(verdict, PermissionVerdict::Ask(_))
}

fn is_deny(verdict: &PermissionVerdict) -> bool {
    matches!(verdict, PermissionVerdict::Deny { .. })
}

#[test]
fn the_mode_table_decides_what_no_rule_covers() {
    let read = json!({"path": "a.txt"});
    let write = json!({"path": "a.txt", "content": ""});
    let outside = json!({"path": "../elsewhere.txt", "content": ""});
    let shell = json!({"command": "git status"});
    let other = json!({});

    let p = policy(PermissionMode::ReadOnly);
    assert_eq!(verdict(&p, "read", read.clone()), PermissionVerdict::Allow);
    assert!(is_deny(&verdict(&p, "write", write.clone())));
    assert!(is_deny(&verdict(&p, "bash", shell.clone())));
    assert!(is_deny(&verdict(&p, "vector_search", other.clone())));

    let p = policy(PermissionMode::Ask);
    assert_eq!(
        verdict(&p, "grep", json!({"pattern": "x"})),
        PermissionVerdict::Allow
    );
    assert!(is_ask(&verdict(&p, "write", write.clone())));
    assert!(is_ask(&verdict(&p, "bash", shell.clone())));
    assert!(is_ask(&verdict(&p, "vector_search", other.clone())));

    let p = policy(PermissionMode::Edits);
    assert_eq!(
        verdict(&p, "write", write.clone()),
        PermissionVerdict::Allow
    );
    assert_eq!(
        verdict(&p, "edit", json!({"path": "src/x.rs"})),
        PermissionVerdict::Allow
    );
    assert!(is_ask(&verdict(&p, "write", outside.clone())));
    assert!(is_ask(&verdict(&p, "bash", shell.clone())));
    assert!(is_ask(&verdict(&p, "vector_search", other.clone())));

    let p = policy(PermissionMode::Auto);
    assert_eq!(verdict(&p, "bash", shell.clone()), PermissionVerdict::Allow);
    assert!(is_ask(&verdict(&p, "write", outside.clone())));
    assert!(is_ask(&verdict(&p, "vector_search", other.clone())));

    let p = policy(PermissionMode::AlwaysApprove);
    assert_eq!(verdict(&p, "bash", shell), PermissionVerdict::Allow);
    assert_eq!(verdict(&p, "write", outside), PermissionVerdict::Allow);
    assert_eq!(
        verdict(&p, "vector_search", other),
        PermissionVerdict::Allow
    );
}

#[test]
fn deny_rules_win_even_in_auto_and_allow_rules_quiet_the_question() {
    let mut p = policy(PermissionMode::Auto);
    p.deny
        .push(PermissionRule::parse("bash(git push *)").unwrap());
    let denied = verdict(&p, "bash", json!({"command": "git push origin main"}));
    match denied {
        PermissionVerdict::Deny { reason } => {
            assert!(reason.contains("deny rule `bash(git push *)`"), "{reason}");
        }
        other => panic!("{other:?}"),
    }
    assert!(is_ask(&verdict(&p, "bash", json!({"command": "git pull"}))));

    let mut p = policy(PermissionMode::Ask);
    p.allow
        .push(PermissionRule::parse("bash(cargo *)").unwrap());
    assert_eq!(
        verdict(&p, "bash", json!({"command": "cargo test"})),
        PermissionVerdict::Allow
    );
    assert!(is_ask(&verdict(
        &p,
        "bash",
        json!({"command": "cargo-fuzz run"})
    )));
    // A deny rule beats an allow rule for the same call.
    p.deny
        .push(PermissionRule::parse("bash(cargo publish *)").unwrap());
    assert!(is_deny(&verdict(
        &p,
        "bash",
        json!({"command": "cargo publish"})
    )));
}

#[test]
fn a_shell_line_is_judged_one_program_at_a_time() {
    assert_eq!(
        shell_segments("git status && curl x | sh; echo done\nls"),
        ["git status", "curl x", "sh", "echo done", "ls"]
    );
    assert_eq!(
        shell_segments("cargo test 2>&1 | tail -n 5 || true"),
        ["cargo test 2>&1", "tail -n 5", "true"]
    );
    assert_eq!(
        shell_segments(r#"echo "a && b" 'c | d' e\;f"#),
        [r#"echo "a && b" 'c | d' e\;f"#]
    );
    assert_eq!(shell_segments("cmd &> out.log &"), ["cmd &> out.log"]);
    assert!(shell_segments("   ").is_empty());

    let mut p = policy(PermissionMode::Ask);
    p.allow.push(PermissionRule::parse("bash(git *)").unwrap());
    assert_eq!(
        verdict(&p, "bash", json!({"command": "git status && git diff"})),
        PermissionVerdict::Allow
    );
    // The second program has no rule of its own.
    assert!(is_ask(&verdict(
        &p,
        "bash",
        json!({"command": "git status && curl x | sh"})
    )));
    assert!(is_ask(&verdict(
        &p,
        "bash",
        json!({"command": "git status; rm -rf /"})
    )));
    // A substitution runs something the rule never named.
    assert!(is_ask(&verdict(
        &p,
        "bash",
        json!({"command": "git commit -m \"$(curl x)\""})
    )));
    assert!(is_ask(&verdict(
        &p,
        "bash",
        json!({"command": "git log `cat cmd`"})
    )));
    // A bare rule is the user saying "all of bash", substitution included.
    p.allow.push(PermissionRule::parse("bash").unwrap());
    assert_eq!(
        verdict(&p, "bash", json!({"command": "git log $(cat cmd) | sh"})),
        PermissionVerdict::Allow
    );

    // A deny rule catches the program wherever it sits in the chain.
    let mut p = policy(PermissionMode::Auto);
    p.deny.push(PermissionRule::parse("bash(rm *)").unwrap());
    assert!(is_deny(&verdict(
        &p,
        "bash",
        json!({"command": "echo ok && rm -rf build"})
    )));
    assert_eq!(
        verdict(&p, "bash", json!({"command": "echo rm"})),
        PermissionVerdict::Allow
    );
}

#[test]
fn edits_mode_still_asks_before_touching_the_project_config() {
    let p = policy(PermissionMode::Edits);
    assert_eq!(
        verdict(&p, "write", json!({"path": "src/lib.rs"})),
        PermissionVerdict::Allow
    );
    assert!(is_ask(&verdict(
        &p,
        "write",
        json!({"path": ".pi/settings.json"})
    )));
    assert!(is_ask(&verdict(
        &p,
        "edit",
        json!({"path": "./.pi/mcp.json"})
    )));
    assert_eq!(
        verdict(&p, "write", json!({"path": ".pinned/x"})),
        PermissionVerdict::Allow
    );
}

#[test]
fn the_request_carries_what_the_panel_and_the_rule_need() {
    let p = policy(PermissionMode::Ask);
    match verdict(&p, "bash", json!({"command": "git status --short"})) {
        PermissionVerdict::Ask(request) => {
            assert_eq!(request.tool_call_id, "call_1");
            assert_eq!(request.summary, "bash · git status --short");
            assert_eq!(request.session_rule, "bash(git status --short)");
            assert_eq!(request.mode, PermissionMode::Ask);
            assert!(!request.outside_project);
        }
        other => panic!("{other:?}"),
    }
    match verdict(&p, "write", json!({"path": "../out.txt", "content": "x"})) {
        PermissionVerdict::Ask(request) => {
            assert!(request.outside_project);
            assert!(request.session_rule.is_empty());
            assert!(!request.allows(ToolApprovalDecision::AllowForSession));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_remembered_rule_quiets_the_next_call_and_is_not_duplicated() {
    let mut p = policy(PermissionMode::Ask);
    let shell = json!({"command": "git status"});
    assert!(is_ask(&verdict(&p, "bash", shell.clone())));
    p.remember("bash(git status *)");
    p.remember("bash(git status *)");
    assert_eq!(p.session_allow.len(), 1);
    assert_eq!(verdict(&p, "bash", shell), PermissionVerdict::Allow);
    assert!(is_ask(&verdict(&p, "bash", json!({"command": "git push"}))));
}

#[test]
fn modes_parse_their_own_names_and_the_codex_sandbox_names() {
    assert_eq!(PermissionMode::parse("ask"), Some(PermissionMode::Ask));
    assert_eq!(
        PermissionMode::parse("READ-ONLY"),
        Some(PermissionMode::ReadOnly)
    );
    assert_eq!(
        PermissionMode::parse("workspace-write"),
        Some(PermissionMode::Edits)
    );
    assert_eq!(
        PermissionMode::parse("full-access"),
        Some(PermissionMode::AlwaysApprove)
    );
    assert_eq!(PermissionMode::parse("nope"), None);
    for mode in PermissionMode::ALL {
        assert_eq!(PermissionMode::parse(mode.as_str()), Some(mode));
    }
}

#[test]
fn the_network_tools_are_asked_about_by_host_and_never_touch_the_workspace() {
    assert_eq!(tool_class("web_fetch"), ToolClass::Network);
    assert_eq!(tool_class("web_search"), ToolClass::Network);
    assert_eq!(tool_class("todo"), ToolClass::Read);
    assert_eq!(tool_class("job_output"), ToolClass::Read);
    assert_eq!(tool_class("notebook_edit"), ToolClass::Edit);
    assert_eq!(tool_class("mcp_read"), ToolClass::Read);
    assert_eq!(tool_class("graph_submit"), ToolClass::Other);
    assert_eq!(tool_class("retrieve_output"), ToolClass::Read);
    assert_eq!(tool_class("graph_run"), ToolClass::Other);
    assert_eq!(tool_class("mcp__memory__echo"), ToolClass::Other);
    assert_eq!(
        host_of("https://user:pw@Docs.rs:443/similar/latest?x=1"),
        "docs.rs"
    );
    assert_eq!(host_of("example.com/page"), "example.com");

    let fetch = json!({"url": "https://docs.rs/similar/latest/"});
    let (subject, outside) = subject_of("web_fetch", &fetch, &cwd());
    assert_eq!(subject, "docs.rs");
    assert!(!outside);
    assert_eq!(
        session_rule_for("web_fetch", &subject).to_string(),
        "web_fetch(docs.rs)"
    );
    assert_eq!(
        session_rule_for("web_search", "rust diff").to_string(),
        "web_search"
    );

    let ask = PermissionPolicy::new(PermissionMode::Ask);
    assert!(matches!(
        ask.decide("c1", "web_fetch", &fetch, &cwd()),
        PermissionVerdict::Ask(request) if request.summary == "web_fetch · docs.rs"
    ));
    let read_only = PermissionPolicy::new(PermissionMode::ReadOnly);
    assert!(matches!(
        read_only.decide("c1", "web_fetch", &fetch, &cwd()),
        PermissionVerdict::Ask(_)
    ));
    assert!(matches!(
        read_only.decide("c1", "web_search", &json!({"query": "rust diff"}), &cwd()),
        PermissionVerdict::Allow
    ));
    let mut read_only_granted = PermissionPolicy::new(PermissionMode::ReadOnly);
    read_only_granted.allow = vec![PermissionRule::parse("web_fetch(docs.rs)").unwrap()];
    assert!(matches!(
        read_only_granted.decide("c1", "web_fetch", &fetch, &cwd()),
        PermissionVerdict::Allow
    ));
    let edits = PermissionPolicy::new(PermissionMode::Edits);
    assert!(matches!(
        edits.decide("c1", "web_search", &json!({"query": "x"}), &cwd()),
        PermissionVerdict::Ask(_)
    ));
    let mut denied = PermissionPolicy::new(PermissionMode::Auto);
    denied.deny = vec![PermissionRule::parse("web_fetch(*.internal)").unwrap()];
    assert!(matches!(
        denied.decide(
            "c1",
            "web_fetch",
            &json!({"url": "http://wiki.corp.internal/x"}),
            &cwd()
        ),
        PermissionVerdict::Deny { .. }
    ));
    let mut granted = PermissionPolicy::new(PermissionMode::Ask);
    granted.allow = vec![PermissionRule::parse("web_fetch(docs.rs)").unwrap()];
    assert!(matches!(
        granted.decide("c1", "web_fetch", &fetch, &cwd()),
        PermissionVerdict::Allow
    ));
}

#[test]
fn mcp_read_only_hint_is_read_and_a_deny_glob_still_wins() {
    let mut policy = PermissionPolicy::new(PermissionMode::Ask);
    policy.mcp_read_only.insert("mcp__memory__echo".into());
    assert_eq!(policy.class_of("mcp__memory__echo"), ToolClass::Read);
    assert!(matches!(
        policy.decide("c1", "mcp__memory__echo", &json!({}), &cwd()),
        PermissionVerdict::Allow
    ));
    assert!(matches!(
        policy.decide("c1", "mcp__memory__write", &json!({}), &cwd()),
        PermissionVerdict::Ask(_)
    ));
    policy.deny = vec![PermissionRule::parse("mcp__memory__*").unwrap()];
    assert!(matches!(
        policy.decide("c1", "mcp__memory__echo", &json!({}), &cwd()),
        PermissionVerdict::Deny { .. }
    ));
    let read_only = PermissionPolicy::new(PermissionMode::ReadOnly);
    assert!(matches!(
        read_only.decide("c1", "mcp__docs__put", &json!({}), &cwd()),
        PermissionVerdict::Deny { .. }
    ));
    assert!(matches!(
        read_only.decide(
            "c1",
            "mcp_read",
            &json!({"server":"memory","uri":"x"}),
            &cwd()
        ),
        PermissionVerdict::Allow
    ));
}

#[test]
fn plan_mode_freezes_mutations_and_keeps_reads() {
    let policy = PermissionPolicy::new(PermissionMode::ReadOnly);
    assert!(matches!(
        policy.decide("c1", "read", &json!({"path": "a.rs"}), &cwd()),
        PermissionVerdict::Allow
    ));
    assert!(matches!(
        policy.decide("c1", "todo", &json!({"items": []}), &cwd()),
        PermissionVerdict::Allow
    ));
    match policy.decide(
        "c1",
        "write",
        &json!({"path": "a.rs", "content": "x"}),
        &cwd(),
    ) {
        PermissionVerdict::Deny { reason } => {
            assert!(reason.contains("plan mode"), "{reason}");
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        policy.decide("c1", "agent", &json!({"prompt": "research"}), &cwd()),
        PermissionVerdict::Allow
    ));
    assert!(matches!(
        policy.decide(
            "c2",
            "agent",
            &json!({"prompt": "edit", "tools": ["read", "write"]}),
            &cwd(),
        ),
        PermissionVerdict::Deny { .. }
    ));
    assert!(matches!(
        policy.decide(
            "c3",
            "agent",
            &json!({"prompt": "research", "isolation": "worktree"}),
            &cwd(),
        ),
        PermissionVerdict::Deny { .. }
    ));
}

#[test]
fn a_worker_on_another_model_needs_the_users_approval_for_that_model() {
    let luna = |mode| PermissionPolicy {
        session_model: Some("openai-codex/gpt-6-luna".into()),
        ..policy(mode)
    };
    let sol = json!({"prompt": "find the root cause", "model": "gpt-6-sol"});

    // Asked in every gated mode, naming the model, with a session grant.
    for mode in [
        PermissionMode::Ask,
        PermissionMode::Edits,
        PermissionMode::Auto,
    ] {
        let PermissionVerdict::Ask(request) = luna(mode).decide("c1", "agent", &sol, &cwd()) else {
            panic!("{mode:?} must ask before a Sol worker");
        };
        assert_eq!(request.subject, "model:openai-codex/gpt-6-sol");
        assert!(request.summary.contains("gpt-6-sol"), "{}", request.summary);
        assert!(
            request.summary.contains("gpt-6-luna"),
            "{}",
            request.summary
        );
        assert_eq!(request.session_rule, "agent(model:openai-codex/gpt-6-sol)");
        assert!(request
            .legal_choices
            .iter()
            .any(|choice| choice.scope == crate::approval::GrantScope::Session));
    }

    // Plan Mode lets read-only workers run unasked, but not on another model.
    let plan = luna(PermissionMode::ReadOnly);
    assert!(matches!(
        plan.decide("c2", "agent", &json!({"prompt": "research"}), &cwd()),
        PermissionVerdict::Allow
    ));
    assert!(is_ask(&plan.decide("c3", "agent", &sol, &cwd())));

    // The grant covers that model only; a broad `agent` grant covers none.
    let mut auto = luna(PermissionMode::Auto);
    auto.remember("agent");
    assert!(is_ask(&auto.decide("c4", "agent", &sol, &cwd())));
    auto.remember("agent(model:openai-codex/gpt-6-sol)");
    assert!(matches!(
        auto.decide("c5", "agent", &sol, &cwd()),
        PermissionVerdict::Allow
    ));
    let full_name = json!({"prompt": "x", "model": "openai-codex/gpt-6-sol"});
    assert!(matches!(
        auto.decide("c6", "agent", &full_name, &cwd()),
        PermissionVerdict::Allow
    ));
    let astra = json!({"prompt": "x", "model": "gpt-6-astra"});
    assert!(is_ask(&auto.decide("c7", "agent", &astra, &cwd())));

    // A batch is decided task by task, so one cross-model task asks for
    // the whole call even under a broad grant.
    let batch = json!({"tasks": [
        {"prompt": "a"},
        {"prompt": "b", "model": "openai-codex/gpt-6-sol"},
    ]});
    let mut broad = luna(PermissionMode::Auto);
    broad.remember("agent");
    assert!(is_ask(&broad.decide("c11", "agent", &batch, &cwd())));
    assert!(is_ask(&luna(PermissionMode::ReadOnly).decide(
        "c12",
        "agent",
        &batch,
        &cwd()
    )));

    // The session's own model is not an escalation; Always Approve is the
    // user's standing yes; with no session model there is nothing to compare.
    let mut same = luna(PermissionMode::Auto);
    same.remember("agent");
    for model in ["gpt-6-luna", "openai-codex/gpt-6-luna"] {
        assert!(matches!(
            same.decide(
                "c8",
                "agent",
                &json!({"prompt": "x", "model": model}),
                &cwd()
            ),
            PermissionVerdict::Allow
        ));
    }
    assert!(matches!(
        luna(PermissionMode::AlwaysApprove).decide("c9", "agent", &sol, &cwd()),
        PermissionVerdict::Allow
    ));
    let mut unknown = policy(PermissionMode::Auto);
    unknown.remember("agent");
    assert!(matches!(
        unknown.decide("c10", "agent", &sol, &cwd()),
        PermissionVerdict::Allow
    ));
}

#[test]
fn a_profile_that_picks_another_model_is_gated_like_an_explicit_model() {
    // Regression (commit security review): `{"agent":"deep"}` ran a profile
    // whose `model:` names Sol without asking, because only `args.model`
    // was compared. Plugin profiles load even in untrusted projects.
    let mut policy = PermissionPolicy {
        session_model: Some("openai-codex/gpt-6-luna".into()),
        ..PermissionPolicy::new(PermissionMode::Auto)
    };
    policy
        .agent_profile_models
        .insert("deep".into(), "openai-codex/gpt-6-sol".into());
    policy
        .agent_profile_models
        .insert("scout".into(), "gpt-6-luna".into());
    policy.remember("agent");

    let PermissionVerdict::Ask(request) = policy.decide(
        "p1",
        "agent",
        &json!({"prompt": "x", "agent": "deep"}),
        &cwd(),
    ) else {
        panic!("a Sol profile must ask");
    };
    assert_eq!(request.subject, "model:openai-codex/gpt-6-sol");
    // Plan Mode's read-only workers do not skip it either.
    let plan = PermissionPolicy {
        mode: PermissionMode::ReadOnly,
        ..policy.clone()
    };
    assert!(is_ask(&plan.decide(
        "p2",
        "agent",
        &json!({"prompt": "x", "agent": "deep"}),
        &cwd()
    )));
    // A profile on the session's own model, or an explicit `model` that
    // overrides the profile with the session's, is not an escalation.
    for args in [
        json!({"prompt": "x", "agent": "scout"}),
        json!({"prompt": "x", "agent": "deep", "model": "gpt-6-luna"}),
        json!({"prompt": "x", "agent": "unknown-profile"}),
    ] {
        assert!(
            matches!(
                policy.decide("p3", "agent", &args, &cwd()),
                PermissionVerdict::Allow
            ),
            "{args}"
        );
    }
    // A case variant is a different id to the host, so it asks.
    assert!(is_ask(&policy.decide(
        "p4",
        "agent",
        &json!({"prompt": "x", "model": "openai-codex/GPT-6-Luna"}),
        &cwd()
    )));
    // The session grant for that model covers the profile route too.
    policy.remember("agent(model:openai-codex/gpt-6-sol)");
    assert!(matches!(
        policy.decide(
            "p5",
            "agent",
            &json!({"prompt": "x", "agent": "deep"}),
            &cwd()
        ),
        PermissionVerdict::Allow
    ));
}

#[test]
fn apply_patch_subject_reports_modified_paths_and_detects_outside() {
    let root = cwd();
    let patch = r#"*** Begin Patch
*** Add File: src/inside.rs
+fn inside() {}
*** End Patch"#;
    let (subject, is_out) = subject_of("apply_patch", &json!({"input": patch}), &root);
    assert_eq!(subject, "src/inside.rs");
    assert!(!is_out);

    let patch_outside = r#"*** Begin Patch
*** Add File: ../outside.rs
+fn outside() {}
*** End Patch"#;
    let (_, is_out_bad) = subject_of("apply_patch", &json!({"input": patch_outside}), &root);
    assert!(is_out_bad);
}

#[test]
fn agent_without_tool_scope_still_requires_approval_outside_plan_mode() {
    let call = json!({"prompt": "inspect and then make any needed changes"});
    for mode in [PermissionMode::Edits, PermissionMode::Auto] {
        let policy = PermissionPolicy::new(mode);
        assert!(
            is_ask(&policy.decide("agent-scope", "agent", &call, &cwd())),
            "{mode:?} must not treat an unscoped child agent as read-only"
        );
    }
}

#[test]
fn agent_isolation_permission_rules() {
    let root = cwd();
    let wt_call = json!({"prompt": "edit", "isolation": "worktree"});
    let (subject, is_out) = subject_of("agent", &wt_call, &root);
    assert_eq!(subject, "isolation:worktree");
    assert!(!is_out);
    assert_eq!(
        session_rule_for("agent", &subject).to_string(),
        "agent(isolation:worktree)"
    );

    let mut policy = PermissionPolicy::new(PermissionMode::Auto);
    policy.deny = vec![PermissionRule::parse("agent(isolation:worktree)").unwrap()];
    assert!(matches!(
        policy.decide("c1", "agent", &wt_call, &root),
        PermissionVerdict::Deny { .. }
    ));

    let shared_call = json!({"prompt": "search", "isolation": "shared"});
    assert!(matches!(
        policy.decide("c2", "agent", &shared_call, &root),
        PermissionVerdict::Ask(_)
    ));
    policy.mode = PermissionMode::AlwaysApprove;
    assert_eq!(
        policy.decide("c3", "agent", &shared_call, &root),
        PermissionVerdict::Allow
    );
    assert!(is_deny(&policy.decide("c4", "agent", &wt_call, &root)));
}

#[test]
fn agent_deny_rule_matches_any_task_in_a_batch() {
    let mut policy = PermissionPolicy::new(PermissionMode::AlwaysApprove);
    policy.deny = vec![PermissionRule::parse("agent(isolation:worktree)").unwrap()];
    let batch = json!({"tasks":[
        {"prompt":"a","isolation":"shared"},
        {"prompt":"b","isolation":"worktree"}
    ]});
    assert!(is_deny(&verdict(&policy, "agent", batch)));
}

#[test]
fn agent_batch_approval_names_every_task_and_is_once_only() {
    let policy = PermissionPolicy::new(PermissionMode::Ask);
    let batch = json!({"tasks":[
        {"prompt":"inspect authentication", "isolation":"shared"},
        {"prompt":"review database writes", "isolation":"shared"}
    ]});
    let PermissionVerdict::Ask(request) = verdict(&policy, "agent", batch) else {
        panic!("expected batch approval")
    };
    assert!(request.summary.contains("inspect authentication"));
    assert!(request.summary.contains("review database writes"));
    assert!(request.session_rule.is_empty());
    assert!(request.allows(ToolApprovalDecision::AllowOnce));
    assert!(!request.allows(ToolApprovalDecision::AllowForSession));
    assert!(!request.allows(ToolApprovalDecision::AllowAlways));
}

#[test]
fn agent_allow_rule_must_match_every_task() {
    let mut policy = PermissionPolicy::new(PermissionMode::Ask);
    policy.allow = vec![PermissionRule::parse("agent(isolation:shared)").unwrap()];
    let mixed = json!({"tasks":[
        {"prompt":"a","isolation":"shared"},
        {"prompt":"b","isolation":"worktree"}
    ]});
    assert!(is_ask(&verdict(&policy, "agent", mixed)));
    let uniform = json!({"tasks":[
        {"prompt":"a","isolation":"shared"},
        {"prompt":"b"}
    ]});
    assert!(matches!(
        verdict(&policy, "agent", uniform),
        PermissionVerdict::Allow
    ));
}

#[test]
fn agent_top_level_fields_beside_tasks_are_task_defaults() {
    let mut policy = PermissionPolicy::new(PermissionMode::Ask);
    policy.allow = vec![PermissionRule::parse("agent(isolation:worktree)").unwrap()];
    // A task's own field wins over the top-level default.
    let own_wins = json!({"isolation":"shared","tasks":[{"prompt":"a","isolation":"worktree"}]});
    assert!(matches!(
        verdict(&policy, "agent", own_wins),
        PermissionVerdict::Allow
    ));
    // The top-level default reaches tasks that do not set the field.
    let inherited = json!({"isolation":"shared","tasks":[{"prompt":"a"}]});
    assert!(is_ask(&verdict(&policy, "agent", inherited)));
}

#[test]
fn agent_empty_tasks_beside_a_prompt_is_a_single_worker() {
    let policy = PermissionPolicy::new(PermissionMode::AlwaysApprove);
    for tasks in [json!([]), serde_json::Value::Null] {
        let call =
            json!({"agent":"scout","description":"orient","prompt":"map the repo","tasks":tasks});
        assert!(matches!(
            verdict(&policy, "agent", call),
            PermissionVerdict::Allow
        ));
    }
}

#[test]
fn agent_tasks_that_are_not_an_array_are_refused() {
    let policy = PermissionPolicy::new(PermissionMode::AlwaysApprove);
    let call = json!({"prompt":"a","tasks":"b"});
    assert!(is_deny(&verdict(&policy, "agent", call)));
}

#[test]
fn deny_parent_checkout_blocks_parent_edits() {
    let parent_dir = cwd();
    let mut policy = PermissionPolicy::new(PermissionMode::Edits);
    policy.deny_parent_checkout(&parent_dir);

    let inside_parent = parent_dir.join("src").join("main.rs");
    let wt_dir = std::env::temp_dir().join("davinci").join("wt-123");
    let call = json!({"path": inside_parent.to_string_lossy().to_string(), "content": "mutated"});
    assert!(matches!(
        policy.decide("c1", "write", &call, &wt_dir),
        PermissionVerdict::Deny { .. }
    ));
}

#[test]
fn rules_parse_parameter_syntax() {
    let r1 = PermissionRule::parse_with_diagnostic("Agent(model:anthropic/*)").unwrap();
    assert_eq!(r1.tool, "Agent");
    assert_eq!(
        r1.specifier,
        Some(RuleSpecifier::Parameter {
            param: "model".into(),
            value: "anthropic/*".into(),
        })
    );
    assert_eq!(r1.to_string(), "Agent(model:anthropic/*)");

    let r2 = PermissionRule::parse_with_diagnostic("Agent(isolation:worktree)").unwrap();
    assert_eq!(
        r2.specifier,
        Some(RuleSpecifier::Parameter {
            param: "isolation".into(),
            value: "worktree".into(),
        })
    );

    let r3 = PermissionRule::parse_with_diagnostic("Bash(run_in_background:true)").unwrap();
    assert_eq!(
        r3.specifier,
        Some(RuleSpecifier::Parameter {
            param: "run_in_background".into(),
            value: "true".into(),
        })
    );

    let r4 = PermissionRule::parse_with_diagnostic("Read(./secrets/**)").unwrap();
    assert_eq!(
        r4.specifier,
        Some(RuleSpecifier::Subject("./secrets/**".into()))
    );

    let r5 = PermissionRule::parse_with_diagnostic("WebFetch(domain:example.com)").unwrap();
    assert_eq!(
        r5.specifier,
        Some(RuleSpecifier::Parameter {
            param: "domain".into(),
            value: "example.com".into(),
        })
    );

    // Windows drive letters must not be parsed as parameter rules
    let r6 = PermissionRule::parse_with_diagnostic("Read(C:/secrets/**)").unwrap();
    assert_eq!(
        r6.specifier,
        Some(RuleSpecifier::Subject("C:/secrets/**".into()))
    );
    let r7 = PermissionRule::parse_with_diagnostic("Read(C:\\secrets\\**)").unwrap();
    assert_eq!(
        r7.specifier,
        Some(RuleSpecifier::Subject("C:\\secrets\\**".into()))
    );

    // URLs with schemes must not be parsed as parameter rules
    let r8 = PermissionRule::parse_with_diagnostic("WebFetch(https://example.com/*)").unwrap();
    assert_eq!(
        r8.specifier,
        Some(RuleSpecifier::Subject("https://example.com/*".into()))
    );

    // Host with port must not be parsed as parameter rule
    let r9 = PermissionRule::parse_with_diagnostic("WebFetch(example.com:8080)").unwrap();
    assert_eq!(
        r9.specifier,
        Some(RuleSpecifier::Subject("example.com:8080".into()))
    );

    // Wildcard and bare
    let r10 = PermissionRule::parse_with_diagnostic("Agent(*)").unwrap();
    assert_eq!(r10.specifier, Some(RuleSpecifier::Wildcard));
    assert_eq!(r10.to_string(), "Agent(*)");

    let r11 = PermissionRule::parse_with_diagnostic("Agent").unwrap();
    assert_eq!(r11.specifier, None);
    assert_eq!(r11.to_string(), "Agent");
}

#[test]
fn rules_parse_malformed_diagnostics() {
    assert_eq!(
        PermissionRule::parse_with_diagnostic(""),
        Err(RuleParseError::Empty)
    );
    assert_eq!(
        PermissionRule::parse_with_diagnostic("   "),
        Err(RuleParseError::Empty)
    );
    assert_eq!(
        PermissionRule::parse_with_diagnostic("Tool("),
        Err(RuleParseError::MissingClosingParen)
    );
    assert_eq!(
        PermissionRule::parse_with_diagnostic("Tool(param:)"),
        Err(RuleParseError::EmptyParameterValue)
    );
    assert_eq!(
        PermissionRule::parse_with_diagnostic("Tool(:value)"),
        Err(RuleParseError::EmptyParameterName)
    );
    assert_eq!(
        PermissionRule::parse_with_diagnostic("Tool(param:value)extra"),
        Err(RuleParseError::TrailingCharacters("extra".into()))
    );
    assert_eq!(
        PermissionRule::parse_with_diagnostic("two words"),
        Err(RuleParseError::InvalidToolName("two words".into()))
    );
    assert_eq!(
        PermissionRule::parse_with_diagnostic("Tool(param:value))"),
        Err(RuleParseError::TrailingCharacters(")".into()))
    );
}

#[test]
fn parameter_aware_wildcard_matching() {
    let root = cwd();
    let mut policy = PermissionPolicy::new(PermissionMode::Ask);
    policy
        .allow
        .push(PermissionRule::parse("Agent(model:*)").unwrap());

    // Model present -> matches
    let call_with_model = json!({"prompt": "hi", "model": "anthropic/claude-3-5-sonnet"});
    assert_eq!(
        policy.decide("c1", "agent", &call_with_model, &root),
        PermissionVerdict::Allow
    );

    // Model omitted -> does not match
    let call_without_model = json!({"prompt": "hi"});
    assert!(matches!(
        policy.decide("c2", "agent", &call_without_model, &root),
        PermissionVerdict::Ask(_)
    ));

    // Agent(*) matches even without model
    let mut wildcard_policy = PermissionPolicy::new(PermissionMode::Ask);
    wildcard_policy
        .allow
        .push(PermissionRule::parse("Agent(*)").unwrap());
    assert_eq!(
        wildcard_policy.decide("c3", "agent", &call_without_model, &root),
        PermissionVerdict::Allow
    );
}

#[test]
fn parameter_aware_omitted_parameter() {
    let root = cwd();
    let mut policy = PermissionPolicy::new(PermissionMode::Ask);
    policy
        .allow
        .push(PermissionRule::parse("Bash(run_in_background:true)").unwrap());

    // Omitted parameter -> does not match
    let call_sync = json!({"command": "cargo test"});
    assert!(matches!(
        policy.decide("c1", "bash", &call_sync, &root),
        PermissionVerdict::Ask(_)
    ));

    // Provided true -> matches
    let call_bg = json!({"command": "cargo test", "run_in_background": true});
    assert_eq!(
        policy.decide("c2", "bash", &call_bg, &root),
        PermissionVerdict::Allow
    );

    // Provided false -> does not match
    let call_not_bg = json!({"command": "cargo test", "run_in_background": false});
    assert!(matches!(
        policy.decide("c3", "bash", &call_not_bg, &root),
        PermissionVerdict::Ask(_)
    ));
}

#[test]
fn parameter_aware_exact_value() {
    let root = cwd();
    let mut policy = PermissionPolicy::new(PermissionMode::Ask);
    policy
        .allow
        .push(PermissionRule::parse("Agent(isolation:worktree)").unwrap());

    let call_wt = json!({"prompt": "edit", "isolation": "worktree"});
    assert_eq!(
        policy.decide("c1", "agent", &call_wt, &root),
        PermissionVerdict::Allow
    );

    let call_shared = json!({"prompt": "edit", "isolation": "shared"});
    assert!(matches!(
        policy.decide("c2", "agent", &call_shared, &root),
        PermissionVerdict::Ask(_)
    ));
}

#[test]
fn deny_precedence_at_every_layer() {
    let root = cwd();

    // 1. Deny beats allow for matching parameter call
    let mut policy = PermissionPolicy::new(PermissionMode::Ask);
    policy
        .allow
        .push(PermissionRule::parse("Agent(model:anthropic/*)").unwrap());
    policy
        .deny
        .push(PermissionRule::parse("Agent(model:anthropic/claude-2)").unwrap());

    let call_allowed = json!({"prompt": "hi", "model": "anthropic/claude-3-5-sonnet"});
    assert_eq!(
        policy.decide("c1", "agent", &call_allowed, &root),
        PermissionVerdict::Allow
    );

    let call_denied = json!({"prompt": "hi", "model": "anthropic/claude-2"});
    assert!(matches!(
        policy.decide("c2", "agent", &call_denied, &root),
        PermissionVerdict::Deny { .. }
    ));

    // 2. Deny beats Auto mode
    let mut auto_policy = PermissionPolicy::new(PermissionMode::Auto);
    auto_policy
        .deny
        .push(PermissionRule::parse("Bash(run_in_background:true)").unwrap());

    let call_bg = json!({"command": "cargo build", "run_in_background": true});
    assert!(matches!(
        auto_policy.decide("c3", "bash", &call_bg, &root),
        PermissionVerdict::Deny { .. }
    ));

    let call_sync = json!({"command": "cargo build --offline"});
    assert!(matches!(
        auto_policy.decide("c4", "bash", &call_sync, &root),
        PermissionVerdict::Ask(_)
    ));
}

#[test]
fn parameter_aware_nested_task_matching() {
    let root = cwd();
    let mut policy = PermissionPolicy::new(PermissionMode::Ask);
    policy
        .allow
        .push(PermissionRule::parse("Agent(model:anthropic/*)").unwrap());
    policy
        .allow
        .push(PermissionRule::parse("Agent(isolation:worktree)").unwrap());

    // Tasks array element matching
    let call_with_tasks = json!({
        "tasks": [
            {"prompt": "research", "model": "anthropic/claude-3-haiku", "isolation": "worktree"}
        ]
    });
    assert_eq!(
        policy.decide("c1", "agent", &call_with_tasks, &root),
        PermissionVerdict::Allow
    );
}

#[test]
fn dedicated_matchers_for_primary_content_fields() {
    let root = cwd();

    // command matcher
    let mut p1 = PermissionPolicy::new(PermissionMode::Ask);
    p1.allow
        .push(PermissionRule::parse("Bash(command:git status *)").unwrap());
    assert_eq!(
        p1.decide(
            "c1",
            "bash",
            &json!({"command": "git status --short"}),
            &root
        ),
        PermissionVerdict::Allow
    );

    // path matcher
    let mut p2 = PermissionPolicy::new(PermissionMode::Ask);
    p2.allow
        .push(PermissionRule::parse("Read(path:src/**)").unwrap());
    assert_eq!(
        p2.decide("c2", "read", &json!({"path": "src/lib.rs"}), &root),
        PermissionVerdict::Allow
    );

    // domain matcher
    let mut p3 = PermissionPolicy::new(PermissionMode::Ask);
    p3.allow
        .push(PermissionRule::parse("WebFetch(domain:*.example.com)").unwrap());
    assert_eq!(
        p3.decide(
            "c3",
            "web_fetch",
            &json!({"url": "https://api.example.com/data"}),
            &root
        ),
        PermissionVerdict::Allow
    );
}

#[test]
fn visual_snapshot_is_a_network_tool_scoped_by_url_host() {
    assert_eq!(tool_class("visual_snapshot"), ToolClass::Network);
    let root = cwd();
    let mut policy = PermissionPolicy::new(PermissionMode::Ask);
    policy
        .allow
        .push(PermissionRule::parse("VisualSnapshot(domain:*.example.com)").unwrap());
    assert_eq!(
        policy.decide(
            "visual-1",
            "visual_snapshot",
            &json!({
                "url": "https://app.example.com",
                "viewportWidth": 1280,
                "viewportHeight": 720
            }),
            &root
        ),
        PermissionVerdict::Allow
    );
    assert!(matches!(
        policy.decide(
            "visual-2",
            "visual_snapshot",
            &json!({
                "url": "https://evil.example.net",
                "viewportWidth": 1280,
                "viewportHeight": 720
            }),
            &root
        ),
        PermissionVerdict::Ask(_)
    ));
}

#[test]
fn windows_drive_and_unc_path_containment() {
    let root = PathBuf::from("C:\\work\\proj");

    // Different drive -> outside
    let (sub, outside) = project_relative(&root, "D:\\other\\file.txt");
    assert!(outside);
    assert_eq!(sub, "D:/other/file.txt");

    // Drive prefix without backslash -> outside
    let (_, outside) = project_relative(&root, "D:other\\file.txt");
    assert!(outside);

    // Same drive inside root -> inside
    let (sub, outside) = project_relative(&root, "C:\\work\\proj\\src\\lib.rs");
    assert!(!outside);
    assert_eq!(sub, "src/lib.rs");

    // UNC path -> outside
    let (sub, outside) = project_relative(&root, "\\\\server\\share\\data.txt");
    assert!(outside);
    assert!(sub.contains("server/share/data.txt"));

    // Normalization above drive root cannot escape drive root
    let norm = normalize_lexically(Path::new("C:\\a\\..\\..\\b"));
    assert_eq!(slashes(&norm), "C:/b");
}

#[test]
fn unix_absolute_and_relative_path_containment() {
    let root = PathBuf::from("/work/proj");

    // Root file -> outside
    let (sub, outside) = project_relative(&root, "/etc/passwd");
    assert!(outside);
    assert_eq!(sub, "/etc/passwd");

    // Directory traversal escaping root -> outside
    let (_, outside) = project_relative(&root, "../../secret");
    assert!(outside);

    // Inside root -> inside
    let (sub, outside) = project_relative(&root, "/work/proj/src/main.rs");
    assert!(!outside);
    assert_eq!(sub, "src/main.rs");

    // Popping root cannot pop below root
    let norm = normalize_lexically(Path::new("/a/../.."));
    assert_eq!(norm, PathBuf::from("/"));
}

#[test]
fn untrusted_symlink_escape_detection() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();

    let secret_file = outside.join("secret.txt");
    std::fs::write(&secret_file, "classified").unwrap();

    // Create a symlink to outside if the platform/privileges allow
    let symlink_dir = root.join("link_to_outside");
    #[cfg(unix)]
    let symlink_created = std::os::unix::fs::symlink(&outside, &symlink_dir).is_ok();
    #[cfg(windows)]
    let symlink_created = std::os::windows::fs::symlink_dir(&outside, &symlink_dir).is_ok();

    if symlink_created {
        let target = symlink_dir.join("secret.txt");
        let (outside_lexical, symlink_escape) = check_path_boundary(&root, &target);
        assert!(
            symlink_escape || outside_lexical,
            "Symlink escape must be detected"
        );

        let (sub, is_out) = project_relative(&root, target.to_str().unwrap());
        assert!(
            is_out,
            "project_relative must mark symlink escape as outside"
        );
        let _ = sub;
    }

    // Internal normal file
    let internal_file = root.join("normal.txt");
    std::fs::write(&internal_file, "hello").unwrap();
    let (outside_lexical, symlink_escape) = check_path_boundary(&root, &internal_file);
    assert!(!outside_lexical);
    assert!(!symlink_escape);
}

#[test]
fn isolated_agent_worktree_boundary_enforcement() {
    let temp = tempfile::tempdir().unwrap();
    let repo_dir = temp.path().join("main_repo");
    let wt_dir = temp.path().join("worktrees").join("wt-1234");
    std::fs::create_dir_all(repo_dir.join("src")).unwrap();
    std::fs::create_dir_all(wt_dir.join("src")).unwrap();

    let repo_file = repo_dir.join("src").join("lib.rs");
    let wt_file = wt_dir.join("src").join("lib.rs");
    std::fs::write(&repo_file, "fn main_repo() {}").unwrap();
    std::fs::write(&wt_file, "fn worktree() {}").unwrap();

    let mut policy = PermissionPolicy::new(PermissionMode::Auto);
    policy.set_worktree_boundary(&wt_dir, Some(&repo_dir));

    // 1. Mutation inside worktree -> Allowed even with strict boundaries
    let wt_write = json!({"path": "src/lib.rs", "content": "updated"});
    assert_eq!(
        policy.decide("c1", "write", &wt_write, &wt_dir),
        PermissionVerdict::Allow
    );

    // 2. Mutation outside worktree targeting main repo -> Strictly Denied even in Auto mode
    let parent_write = json!({
        "path": repo_file.to_string_lossy().to_string(),
        "content": "illegal modification"
    });
    assert!(matches!(
        policy.decide("c2", "write", &parent_write, &wt_dir),
        PermissionVerdict::Deny { .. }
    ));

    // 3. Mutation targeting arbitrary external dir -> Denied
    let external_file = temp.path().join("external.txt");
    let ext_write = json!({
        "path": external_file.to_string_lossy().to_string(),
        "content": "escaped write"
    });
    assert!(matches!(
        policy.decide("c3", "write", &ext_write, &wt_dir),
        PermissionVerdict::Deny { .. }
    ));
}

#[test]
fn read_outside_root_policy_enforcement() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let outside_dir = temp.path().join("outside");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&outside_dir).unwrap();

    let outside_file = outside_dir.join("config.sys");
    std::fs::write(&outside_file, "secret").unwrap();

    let read_outside = json!({"path": outside_file.to_string_lossy().to_string()});

    // 1. Allow policy
    let mut p_allow = PermissionPolicy::new(PermissionMode::Ask);
    p_allow.set_read_outside_root_policy(ReadOutsideRootPolicy::Allow);
    assert_eq!(
        p_allow.decide("c1", "read", &read_outside, &root),
        PermissionVerdict::Allow
    );

    // 2. Ask policy
    let mut p_ask = PermissionPolicy::new(PermissionMode::Ask);
    p_ask.set_read_outside_root_policy(ReadOutsideRootPolicy::Ask);
    assert!(matches!(
        p_ask.decide("c2", "read", &read_outside, &root),
        PermissionVerdict::Ask(_)
    ));

    // 3. Deny policy
    let mut p_deny = PermissionPolicy::new(PermissionMode::Ask);
    p_deny.set_read_outside_root_policy(ReadOutsideRootPolicy::Deny);
    assert!(matches!(
        p_deny.decide("c3", "read", &read_outside, &root),
        PermissionVerdict::Deny { .. }
    ));
}

#[test]
fn git_metadata_access_preserved() {
    let temp = tempfile::tempdir().unwrap();
    let repo_dir = temp.path().join("repo");
    let wt_dir = temp.path().join("worktrees").join("wt-1");
    std::fs::create_dir_all(repo_dir.join(".git").join("worktrees").join("wt-1")).unwrap();
    std::fs::create_dir_all(wt_dir.join("src")).unwrap();

    let mut policy = PermissionPolicy::new(PermissionMode::Edits);
    policy.set_worktree_boundary(&wt_dir, Some(&repo_dir));
    policy.set_read_outside_root_policy(ReadOutsideRootPolicy::Deny);

    // Reading git metadata from repo .git -> Allowed despite ReadOutsideRootPolicy::Deny and deny_parent_checkout
    let git_meta_file = repo_dir
        .join(".git")
        .join("worktrees")
        .join("wt-1")
        .join("gitdir");
    std::fs::write(&git_meta_file, "gitdir").unwrap();
    let read_git_meta = json!({"path": git_meta_file.to_string_lossy().to_string()});
    assert_eq!(
        policy.decide("c1", "read", &read_git_meta, &wt_dir),
        PermissionVerdict::Allow
    );

    // Non-git-metadata file in repo -> Denied
    let non_git_file = repo_dir.join("README.md");
    std::fs::write(&non_git_file, "readme").unwrap();
    let read_non_git = json!({"path": non_git_file.to_string_lossy().to_string()});
    assert!(matches!(
        policy.decide("c2", "read", &read_non_git, &wt_dir),
        PermissionVerdict::Deny { .. }
    ));
}

/// Canonical `workspace`, `extra` and `other` directories beside each other,
/// with a policy rooted at the workspace and `extra` as an additional root.
struct ExtraRoots {
    _temp: tempfile::TempDir,
    workspace: PathBuf,
    extra: PathBuf,
    other: PathBuf,
}

impl ExtraRoots {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let base = strip_verbatim_prefix(&temp.path().canonicalize().unwrap());
        let [workspace, extra, other] = ["workspace", "extra", "other"].map(|name| {
            let dir = base.join(name);
            std::fs::create_dir_all(dir.join("src")).unwrap();
            dir
        });
        Self {
            _temp: temp,
            workspace,
            extra,
            other,
        }
    }

    fn policy(&self, mode: PermissionMode) -> PermissionPolicy {
        let mut policy = PermissionPolicy::new(mode);
        policy.filesystem_boundary.root = Some(self.workspace.clone());
        policy.filesystem_boundary.extra_roots = vec![self.extra.clone()];
        policy
    }

    fn write(&self, policy: &PermissionPolicy, path: &Path) -> PermissionVerdict {
        policy.decide(
            "extra",
            "write",
            &json!({"path": path.to_string_lossy(), "content": "x"}),
            &self.workspace,
        )
    }
}

#[test]
fn extra_root_edits_are_inside_and_other_directories_still_ask() {
    let dirs = ExtraRoots::new();
    for mode in [PermissionMode::Edits, PermissionMode::Auto] {
        let policy = dirs.policy(mode);
        assert_eq!(
            dirs.write(&policy, &dirs.extra.join("x.rs")),
            PermissionVerdict::Allow,
            "{mode:?}"
        );
        assert!(
            is_ask(&dirs.write(&policy, &dirs.other.join("x.rs"))),
            "{mode:?}"
        );
    }
    // The same policy without the extra root treats it as outside.
    let mut plain = dirs.policy(PermissionMode::Edits);
    plain.filesystem_boundary.extra_roots.clear();
    assert!(is_ask(&dirs.write(&plain, &dirs.extra.join("x.rs"))));
}

#[test]
fn extra_root_reads_do_not_ask_in_auto() {
    let dirs = ExtraRoots::new();
    let policy = dirs.policy(PermissionMode::Auto);
    let read = |path: &Path| {
        policy.decide(
            "read",
            "read",
            &json!({"path": path.to_string_lossy()}),
            &dirs.workspace,
        )
    };
    assert_eq!(
        read(&dirs.extra.join("src/lib.rs")),
        PermissionVerdict::Allow
    );
    assert!(is_ask(&read(&dirs.other.join("src/lib.rs"))));
}

#[test]
fn extra_root_subjects_stay_absolute_so_relative_rules_do_not_match() {
    let dirs = ExtraRoots::new();
    let mut policy = dirs.policy(PermissionMode::Ask);
    policy
        .allow
        .push(PermissionRule::parse("write(src/*)").unwrap());
    assert_eq!(
        dirs.write(&policy, &dirs.workspace.join("src/x.rs")),
        PermissionVerdict::Allow
    );
    let target = dirs.extra.join("src").join("x.rs");
    match dirs.write(&policy, &target) {
        PermissionVerdict::Ask(request) => {
            assert_eq!(request.subject, slashes(&target));
            assert!(!request.outside_project);
        }
        other => panic!("expected an ask, got {other:?}"),
    }
    let (subject, outside) = project_relative_with_boundary(
        &dirs.workspace,
        &target.to_string_lossy(),
        Some(&policy.filesystem_boundary),
    );
    assert_eq!((subject, outside), (slashes(&target), false));
}

#[test]
fn protected_paths_in_extra_roots_need_an_exact_grant() {
    let dirs = ExtraRoots::new();
    std::fs::create_dir_all(dirs.extra.join(".git")).unwrap();
    let config = dirs.extra.join(".git").join("config");
    let mut policy = dirs.policy(PermissionMode::Edits);
    assert!(is_ask(&dirs.write(&policy, &config)));

    // The absolute-glob workaround and a relative rule both fall short.
    policy.allow.push(PermissionRule::subject(
        "write",
        format!("{}/**", slashes(&dirs.extra)),
    ));
    policy
        .allow
        .push(PermissionRule::parse("write(.git/*)").unwrap());
    policy.allow.push(PermissionRule::bare("write"));
    assert!(is_ask(&dirs.write(&policy, &config)));
    assert!(is_ask(&dirs.write(&policy, &dirs.extra.join(".env"))));
    // The glob still covers ordinary files in that root.
    assert_eq!(
        dirs.write(&policy, &dirs.extra.join("src/y.rs")),
        PermissionVerdict::Allow
    );

    // Naming the exact file is the user's explicit choice.
    policy
        .allow
        .push(PermissionRule::subject("write", slashes(&config)));
    assert_eq!(dirs.write(&policy, &config), PermissionVerdict::Allow);
}

#[test]
fn symlinks_escaping_the_workspace_or_an_extra_root_are_denied() {
    let dirs = ExtraRoots::new();
    #[cfg(unix)]
    let link = |from: &Path, to: &Path| std::os::unix::fs::symlink(to, from).is_ok();
    // Without the symlink privilege, a junction resolves the same way.
    #[cfg(windows)]
    let link = |from: &Path, to: &Path| {
        std::os::windows::fs::symlink_dir(to, from).is_ok()
            || std::process::Command::new("cmd")
                .arg("/C")
                .arg("mklink")
                .arg("/J")
                .arg(from)
                .arg(to)
                .output()
                .is_ok_and(|out| out.status.success())
    };
    if !link(&dirs.workspace.join("link"), &dirs.other) {
        return; // No way to link on this host.
    }
    assert!(link(&dirs.extra.join("link"), &dirs.other));
    for mode in PermissionMode::ALL {
        let mut policy = dirs.policy(mode);
        policy.allow.push(PermissionRule::bare("*"));
        for target in [
            dirs.workspace.join("link").join("x.rs"),
            dirs.extra.join("link").join("x.rs"),
        ] {
            assert!(
                is_deny(&dirs.write(&policy, &target)),
                "{mode:?} {}",
                target.display()
            );
        }
    }
}

#[test]
fn isolated_worktree_boundaries_ignore_extra_roots() {
    let dirs = ExtraRoots::new();
    // Set before isolating: cleared.
    let mut before = dirs.policy(PermissionMode::AlwaysApprove);
    before.set_worktree_boundary(&dirs.workspace, None);
    assert!(before.filesystem_boundary.extra_roots.is_empty());
    assert!(is_deny(&dirs.write(&before, &dirs.extra.join("x.rs"))));
    // Set after isolating: ignored.
    let mut after = PermissionPolicy::new(PermissionMode::AlwaysApprove);
    after.set_worktree_boundary(&dirs.workspace, None);
    after.filesystem_boundary.extra_roots = vec![dirs.extra.clone()];
    assert!(after.filesystem_boundary.active_extra_roots().is_empty());
    assert!(is_deny(&dirs.write(&after, &dirs.extra.join("x.rs"))));
}

#[test]
fn auto_shell_workdir_inside_an_extra_root_is_routine() {
    let dirs = ExtraRoots::new();
    let policy = dirs.policy(PermissionMode::Auto);
    let shell = |workdir: &Path| {
        policy.decide(
            "shell",
            "bash",
            &json!({"command": "cat src/lib.rs", "workdir": workdir.to_string_lossy()}),
            &dirs.workspace,
        )
    };
    assert_eq!(shell(&dirs.extra), PermissionVerdict::Allow);
    assert!(is_ask(&shell(&dirs.other)));
    assert!(is_ask(&shell(&dirs.extra.join(".git"))));

    // Path arguments from the workspace follow the same boundary.
    let command = |command: &str| {
        policy.decide(
            "shell",
            "bash",
            &json!({ "command": command }),
            &dirs.workspace,
        )
    };
    assert_eq!(command("cat ../extra/src/lib.rs"), PermissionVerdict::Allow);
    assert!(is_ask(&command("cat ../other/src/lib.rs")));
    assert!(is_ask(&command("cat ../extra/.git/config")));
    let mut plain = dirs.policy(PermissionMode::Auto);
    plain.filesystem_boundary.extra_roots.clear();
    assert!(is_ask(&plain.decide(
        "shell",
        "bash",
        &json!({"command": "cat ../extra/src/lib.rs"}),
        &dirs.workspace,
    )));
}

#[test]
fn validate_extra_root_accepts_project_directories_and_rejects_the_rest() {
    let temp = tempfile::tempdir().unwrap();
    let base = strip_verbatim_prefix(&temp.path().canonicalize().unwrap());
    let home = base.join("home");
    let cwd = home.join("work").join("app");
    let lib = home.join("work").join("lib");
    for dir in [&cwd, &lib, &home.join(".ssh").join("keys")] {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(home.join("work").join("file.txt"), "x").unwrap();
    let check = |raw: &str| validate_extra_root(raw, &cwd, Some(&home));

    assert_eq!(check("../lib").unwrap(), lib);
    assert_eq!(check(&lib.to_string_lossy()).unwrap(), lib);
    assert_eq!(check("~/work/lib").unwrap(), lib);

    let error = |raw: &str| check(raw).unwrap_err();
    assert!(
        error("../missing").contains("does not exist"),
        "{}",
        error("../missing")
    );
    assert!(error("../file.txt").contains("is not a directory"));
    assert!(error(".").contains("already the workspace"));
    assert!(error(&cwd.to_string_lossy()).contains("already the workspace"));
    assert!(error("~").contains("home directory"));
    assert!(error(&base.to_string_lossy()).contains("contains"));
    assert!(error("~/.ssh").contains("contains"));
    assert!(error("~/.ssh/keys").contains("is inside"));
    assert!(error("  ").contains("empty"));
    let filesystem_root = base.ancestors().last().unwrap().to_path_buf();
    assert!(
        validate_extra_root(&filesystem_root.to_string_lossy(), &cwd, None)
            .unwrap_err()
            .contains("filesystem root")
    );
}
