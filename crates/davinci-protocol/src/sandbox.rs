#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn required_capabilities_fail_closed() {
        let available = SandboxCapabilities {
            filesystem_isolation: true,
            network_denied: false,
            environment_isolation: true,
            process_tree_isolation: true,
            timeout: true,
            output_limit: true,
            ..Default::default()
        };
        let required = SandboxCapabilities {
            filesystem_isolation: true,
            network_denied: true,
            ..Default::default()
        };
        assert!(!available.satisfies(&required));
    }

    #[test]
    fn child_capabilities_are_intersection_only() {
        let parent = SandboxCapabilities {
            filesystem_isolation: true,
            network_denied: true,
            environment_isolation: true,
            process_tree_isolation: true,
            memory_limit: true,
            ..Default::default()
        };
        let requested = SandboxCapabilities {
            filesystem_isolation: true,
            network_denied: false,
            environment_isolation: true,
            process_tree_isolation: true,
            cpu_limit: true,
            ..Default::default()
        };
        let child = parent.intersection(&requested);
        assert!(child.is_subset_of(&parent));
        assert!(child.filesystem_isolation);
        assert!(!child.network_denied);
        assert!(!child.cpu_limit);
    }

    #[test]
    fn mount_traversal_is_rejected() {
        let mut spec = fixture_spec();
        spec.filesystem.mounts.push(MountRule {
            source: Some("/host/workspace".into()),
            target: "/project/../escape".into(),
            access: MountAccess::ReadWrite,
        });
        let error = spec.validate().unwrap_err();
        assert_eq!(error.code, SandboxErrorCode::PolicyDenied);
    }

    #[test]
    fn allowlist_requires_real_destinations() {
        let mut spec = fixture_spec();
        spec.network = NetworkPolicy::AllowList {
            domains: vec!["".into(), "example.com".into()],
            ports: vec![443],
        };
        assert_eq!(
            spec.validate().unwrap_err().code,
            SandboxErrorCode::PolicyDenied
        );
    }

    #[test]
    fn environment_names_are_validated() {
        let mut spec = fixture_spec();
        spec.environment.inject = BTreeMap::from([("BAD=NAME".into(), "secret".into())]);
        assert_eq!(
            spec.validate().unwrap_err().code,
            SandboxErrorCode::PolicyDenied
        );
    }

    #[test]
    fn security_types_reject_unknown_fields() {
        let value = serde_json::json!({
            "id": "sbx-1",
            "mode": "restricted",
            "backend": "auto",
            "workspace": "/workspace",
            "filesystem": {"mounts": []},
            "network": {"mode": "denied"},
            "environment": {"allow": [], "inject": {}},
            "resources": {},
            "process": {},
            "required_capabilities": {},
            "model_says_full_access": true
        });
        assert!(serde_json::from_value::<SandboxSpec>(value).is_err());
    }

    fn fixture_spec() -> SandboxSpec {
        SandboxSpec {
            id: SandboxId("sbx-1".into()),
            mode: SandboxMode::Restricted,
            backend: SandboxBackendKind::Auto,
            workspace: "/workspace".into(),
            filesystem: FilesystemPolicy::default(),
            network: NetworkPolicy::Denied,
            environment: EnvironmentPolicy::default(),
            resources: ResourcePolicy::default(),
            process: ProcessPolicy::default(),
            required_capabilities: SandboxCapabilities {
                filesystem_isolation: true,
                network_denied: true,
                environment_isolation: true,
                process_tree_isolation: true,
                ..Default::default()
            },
        }
    }
}
