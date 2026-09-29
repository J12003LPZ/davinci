#[cfg(test)]
mod tests {
    use super::*;
    use davinci_protocol::{
        EnvironmentPolicy, FilesystemPolicy, MountAccess, MountRule, NetworkPolicy,
        ProcessPolicy, ResourcePolicy, SandboxBackendKind, SandboxCapabilities, SandboxId,
        SandboxMode, SandboxSpec,
    };
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    fn ambient_secrets_are_not_in_the_default_environment() {
        let parent = BTreeMap::from([
            ("PATH".into(), "/bin".into()),
            ("LANG".into(), "C.UTF-8".into()),
            ("GITHUB_TOKEN".into(), "secret".into()),
            ("AWS_SECRET_ACCESS_KEY".into(), "secret".into()),
            ("SSH_AUTH_SOCK".into(), "/tmp/agent.sock".into()),
            ("PRIVATE_VALUE".into(), "private".into()),
        ]);
        let policy = EnvironmentPolicy::default();
        let child = sanitize_environment_from(&policy, &parent).unwrap();
        assert_eq!(child.get("PATH").map(String::as_str), Some("/bin"));
        assert_eq!(child.get("LANG").map(String::as_str), Some("C.UTF-8"));
        assert!(!child.contains_key("GITHUB_TOKEN"));
        assert!(!child.contains_key("AWS_SECRET_ACCESS_KEY"));
        assert!(!child.contains_key("SSH_AUTH_SOCK"));
        assert!(!child.contains_key("PRIVATE_VALUE"));
    }

    #[test]
    fn child_policy_cannot_enable_network_denied_by_parent() {
        let parent = spec(SandboxMode::WorkspaceWrite, NetworkPolicy::Denied);
        let child = spec(SandboxMode::WorkspaceWrite, NetworkPolicy::Unrestricted);
        let error = attenuate_child_spec(&parent, &child).unwrap_err();
        assert_eq!(
            error.code,
            davinci_protocol::SandboxErrorCode::PolicyDenied
        );
    }

    #[test]
    fn host_backend_is_only_for_full_access() {
        let backend = HostBackend;
        let spec = spec(SandboxMode::WorkspaceWrite, NetworkPolicy::Denied);
        let request = request(&spec);
        let error = backend
            .prepare(&spec, &request, &BTreeMap::new())
            .unwrap_err();
        assert_eq!(
            error.code,
            davinci_protocol::SandboxErrorCode::CapabilityUnavailable
        );
    }

    #[test]
    fn bubblewrap_denied_network_uses_network_namespace() {
        let backend = LinuxBubblewrapBackend::new(PathBuf::from("/usr/bin/bwrap"));
        let spec = spec(SandboxMode::WorkspaceWrite, NetworkPolicy::Denied);
        let request = request(&spec);
        let prepared = backend
            .prepare(&spec, &request, &BTreeMap::from([("PATH".into(), "/usr/bin".into())]))
            .unwrap();
        assert_eq!(prepared.executable, PathBuf::from("/usr/bin/bwrap"));
        assert!(prepared.argv.iter().any(|arg| arg == "--unshare-net"));
        assert!(prepared.argv.windows(3).any(|args| {
            args == [String::from("--bind"), spec.workspace.clone(), spec.workspace.clone()]
        }));
    }

    #[test]
    fn bubblewrap_does_not_claim_domain_allowlisting() {
        let backend = LinuxBubblewrapBackend::new(PathBuf::from("/usr/bin/bwrap"));
        let mut spec = spec(
            SandboxMode::WorkspaceWrite,
            NetworkPolicy::AllowList {
                domains: vec!["example.com".into()],
                ports: vec![443],
            },
        );
        spec.required_capabilities.network_allowlist = true;
        let error = backend
            .prepare(&spec, &request(&spec), &BTreeMap::new())
            .unwrap_err();
        assert_eq!(
            error.code,
            davinci_protocol::SandboxErrorCode::CapabilityUnavailable
        );
    }

    #[test]
    fn no_execution_mode_fails_before_backend_spawn() {
        let broker = SandboxBroker::default();
        let spec = spec(SandboxMode::NoExecution, NetworkPolicy::Denied);
        let error = broker
            .prepare(&spec, &request(&spec), &BTreeMap::new())
            .unwrap_err();
        assert_eq!(
            error.code,
            davinci_protocol::SandboxErrorCode::PolicyDenied
        );
    }

    fn request(spec: &SandboxSpec) -> davinci_protocol::ExecutionRequest {
        davinci_protocol::ExecutionRequest {
            sandbox_id: spec.id.clone(),
            executable: if cfg!(windows) {
                "C:\\Windows\\System32\\cmd.exe".into()
            } else {
                "/bin/sh".into()
            },
            argv: vec!["-c".into(), "true".into()],
            cwd: spec.workspace.clone(),
        }
    }

    fn spec(mode: SandboxMode, network: NetworkPolicy) -> SandboxSpec {
        let workspace = if cfg!(windows) {
            "C:\\workspace".to_string()
        } else {
            "/workspace".to_string()
        };
        SandboxSpec {
            id: SandboxId("sbx-test".into()),
            mode,
            backend: if mode == SandboxMode::FullAccess {
                SandboxBackendKind::Host
            } else {
                SandboxBackendKind::LinuxBubblewrap
            },
            workspace: workspace.clone(),
            filesystem: FilesystemPolicy {
                mounts: vec![MountRule {
                    source: Some(workspace.clone()),
                    target: workspace,
                    access: if mode == SandboxMode::Restricted {
                        MountAccess::ReadOnly
                    } else {
                        MountAccess::ReadWrite
                    },
                }],
            },
            network,
            environment: EnvironmentPolicy::default(),
            resources: ResourcePolicy::default(),
            process: ProcessPolicy::default(),
            required_capabilities: SandboxCapabilities {
                filesystem_isolation: mode != SandboxMode::FullAccess,
                network_denied: mode != SandboxMode::FullAccess,
                environment_isolation: true,
                process_tree_isolation: true,
                deterministic_teardown: true,
                ..Default::default()
            },
        }
    }
}
