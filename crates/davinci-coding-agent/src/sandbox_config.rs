#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_policy_can_only_narrow_global_authority() {
        let global = SandboxSettings {
            mode: Some("workspace_write".into()),
            backend: Some("auto".into()),
            network: Some(SandboxNetworkSettings {
                mode: Some("deny".into()),
                ..Default::default()
            }),
            resources: Some(SandboxResourceSettings {
                timeout_seconds: Some(300),
                max_memory_mb: Some(4096),
                max_processes: Some(128),
            }),
        };
        let project = SandboxSettings {
            mode: Some("restricted".into()),
            backend: None,
            network: Some(SandboxNetworkSettings {
                mode: Some("deny".into()),
                ..Default::default()
            }),
            resources: Some(SandboxResourceSettings {
                timeout_seconds: Some(120),
                max_memory_mb: Some(2048),
                max_processes: Some(64),
            }),
        };
        let root = tempfile::tempdir().unwrap();
        let spec = resolve_sandbox_settings(root.path(), Some(&global), Some(&project), true)
            .unwrap()
            .unwrap();
        assert_eq!(spec.mode, davinci_protocol::SandboxMode::Restricted);
        assert_eq!(spec.resources.timeout_ms, Some(120_000));
        assert_eq!(spec.resources.max_memory_bytes, Some(2048 * 1024 * 1024));
        assert_eq!(spec.resources.max_processes, Some(64));
    }

    #[test]
    fn project_cannot_enable_full_access_or_unrestricted_network() {
        let global = SandboxSettings {
            mode: Some("workspace_write".into()),
            ..Default::default()
        };
        let project = SandboxSettings {
            mode: Some("full_access".into()),
            network: Some(SandboxNetworkSettings {
                mode: Some("unrestricted".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let root = tempfile::tempdir().unwrap();
        let error = resolve_sandbox_settings(root.path(), Some(&global), Some(&project), true)
            .unwrap_err();
        assert!(error.contains("cannot widen"), "{error}");
    }

    #[test]
    fn untrusted_project_sandbox_settings_are_ignored() {
        let global = SandboxSettings {
            mode: Some("restricted".into()),
            ..Default::default()
        };
        let project = SandboxSettings {
            mode: Some("full_access".into()),
            ..Default::default()
        };
        let root = tempfile::tempdir().unwrap();
        let spec = resolve_sandbox_settings(root.path(), Some(&global), Some(&project), false)
            .unwrap()
            .unwrap();
        assert_eq!(spec.mode, davinci_protocol::SandboxMode::Restricted);
    }

    #[test]
    fn absent_global_sandbox_keeps_legacy_execution_explicitly_disabled() {
        let root = tempfile::tempdir().unwrap();
        assert!(resolve_sandbox_settings(root.path(), None, None, false)
            .unwrap()
            .is_none());
    }

    #[test]
    fn workspace_write_defaults_to_denied_network_and_minimal_home() {
        let root = tempfile::tempdir().unwrap();
        let spec = resolve_sandbox_settings(
            root.path(),
            Some(&SandboxSettings {
                mode: Some("workspace_write".into()),
                ..Default::default()
            }),
            None,
            false,
        )
        .unwrap()
        .unwrap();
        assert!(matches!(
            spec.network,
            davinci_protocol::NetworkPolicy::Denied
        ));
        assert_eq!(
            spec.environment.inject.get("HOME").map(String::as_str),
            Some("/tmp/davinci-home")
        );
        assert!(spec.required_capabilities.filesystem_isolation);
        assert!(spec.required_capabilities.network_denied);
        assert!(spec.required_capabilities.environment_isolation);
    }
}
