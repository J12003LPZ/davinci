//! Native DaVinci policy, intentionally not mirrored from TypeScript.
//! Seatbelt filters host paths; it cannot relocate mounts or provide a new root.
use super::*;

#[derive(Debug, Clone)]
pub struct MacosSeatbeltBackend {
    private_temp: Option<PathBuf>,
}

impl MacosSeatbeltBackend {
    pub fn new(private_temp: Option<&Path>) -> Self {
        Self {
            private_temp: private_temp.map(Path::to_path_buf),
        }
    }

    fn profile(
        &self,
        spec: &SandboxSpec,
        request: &ExecutionRequest,
    ) -> Result<(String, PathBuf), SandboxFailure> {
        let workspace = canonical(Path::new(&spec.workspace))?;
        if workspace != Path::new(&spec.workspace) {
            return Err(SandboxFailure::policy_denied(
                "Seatbelt requires a canonical workspace",
            ));
        }
        if !canonical(Path::new(&request.cwd))?.starts_with(&workspace) {
            return Err(SandboxFailure::new(
                SandboxErrorCode::FilesystemDenied,
                "Seatbelt cwd resolves outside workspace",
            ));
        }
        let private_temp = canonical(self.private_temp.as_deref().ok_or_else(|| {
            SandboxFailure::capability_unavailable(
                "Seatbelt requires supervisor-owned private temp",
            )
        })?)?;
        if private_temp.starts_with(&workspace) || workspace.starts_with(&private_temp) {
            return Err(SandboxFailure::policy_denied(
                "Seatbelt temp must be separate from workspace",
            ));
        }
        let workspace_access = if spec.mode == SandboxMode::Restricted {
            MountAccess::ReadOnly
        } else {
            MountAccess::ReadWrite
        };
        if !spec.filesystem.mounts.iter().any(|mount| {
            mount.source.as_deref() == Some(spec.workspace.as_str())
                && mount.target == spec.workspace
                && mount.access == workspace_access
        }) {
            return Err(SandboxFailure::new(
                SandboxErrorCode::FilesystemDenied,
                "Seatbelt requires an exact workspace mount",
            ));
        }
        let mut read = Vec::new();
        let mut write = vec![private_temp.clone()];
        let mut readonly = Vec::new();
        let mut hidden = Vec::new();
        for mount in &spec.filesystem.mounts {
            match mount.access {
                MountAccess::ReadOnly | MountAccess::ReadWrite => {
                    let source = canonical(Path::new(
                        mount.source.as_deref().expect("validated source"),
                    ))?;
                    if source != canonical(Path::new(&mount.target))? {
                        return Err(SandboxFailure::capability_unavailable(
                            "Seatbelt cannot relocate host mounts",
                        ));
                    }
                    read.push(source.clone());
                    if mount.access == MountAccess::ReadWrite {
                        write.push(source);
                    } else {
                        readonly.push(source);
                    }
                }
                MountAccess::Temporary => {
                    // Only this conventional logical home is supported; the
                    // supervisor substitutes its private launch directory.
                    if mount.target != "/tmp/davinci-home" {
                        return Err(SandboxFailure::capability_unavailable(
                            "Seatbelt supports only the private launch home temporary mount",
                        ));
                    }
                }
                MountAccess::Hidden => {
                    let lexical = PathBuf::from(&mount.target);
                    hidden.push(lexical.clone());
                    if let Ok(path) = lexical.canonicalize() {
                        hidden.push(path);
                    }
                }
            }
        }
        read.push(private_temp.clone());
        for name in [".git", ".davinci", ".pi"] {
            // Protect even absent roots, and symlink targets inside otherwise
            // writable trees. Granting parent access must never override this.
            let path = workspace.join(name);
            readonly.push(path.clone());
            if let Ok(target) = path.canonicalize() {
                readonly.push(target);
            }
        }
        let mut profile = String::from("(version 1)\n(deny default)\n(allow process-fork)\n(allow sysctl-read)\n(allow file-read-metadata)\n");
        // macOS 26 dyld's CacheFinder opens the root directory before main.
        // Native failure evidence reports file-read-data / and an ignition
        // abort. Match only that directory, never files beneath the root.
        profile.push_str("(allow file-read-data (literal \"/\"))\n");
        // POSIX tools need these specific devices, never the entire /dev tree.
        profile.push_str("(allow file-read* file-write* (literal \"/dev/null\") (literal \"/dev/zero\"))\n(allow file-read* (literal \"/dev/random\") (literal \"/dev/urandom\"))\n");
        grant(
            &mut profile,
            "file-read* file-map-executable process-exec",
            &read,
            &hidden,
        )?;
        for path in &hidden {
            profile.push_str(&format!(
                "(deny file-read* file-write* file-map-executable (subpath {}))\n",
                quote(path)?
            ));
        }
        let mut denied_writes = readonly;
        denied_writes.extend(hidden.iter().cloned());
        grant(&mut profile, "file-write*", &write, &denied_writes)?;
        // The child cannot replace/remove the owned root, or its home. A host
        // cleanup can remove it without a descendant recreating the root.
        profile.push_str(&format!(
            "(deny file-write* (literal {}) (literal {}))\n",
            quote(&private_temp)?,
            quote(&private_temp.join("home"))?
        ));
        if matches!(spec.network, NetworkPolicy::Unrestricted) {
            profile.push_str("(allow network*)\n");
        }
        Ok((profile, private_temp))
    }
}

impl SandboxBackend for MacosSeatbeltBackend {
    fn kind(&self) -> SandboxBackendKind {
        SandboxBackendKind::MacosSeatbelt
    }
    fn capabilities(&self) -> SandboxCapabilities {
        SandboxCapabilities {
            filesystem_isolation: true,
            network_denied: true,
            environment_isolation: true,
            output_limit: true,
            // Seatbelt does not mediate setsid/setpgid. The Unix supervisor
            // kills its process group, so detached descendants are not owned.
            // Do not advertise tree timeout/teardown or lifetime-private temp.
            // RLIMIT_DATA is not hard memory confinement on macOS. PID and
            // temp size enforcement are unavailable, too.
            ..Default::default()
        }
    }
    fn prepare(
        &self,
        spec: &SandboxSpec,
        request: &ExecutionRequest,
        environment: &BTreeMap<String, String>,
    ) -> Result<PreparedExecution, SandboxFailure> {
        validate_request(spec, request)?;
        if !matches!(
            spec.mode,
            SandboxMode::Restricted | SandboxMode::WorkspaceWrite
        ) {
            return Err(SandboxFailure::capability_unavailable(
                "Seatbelt is for restricted/workspace_write execution",
            ));
        }
        if matches!(spec.network, NetworkPolicy::AllowList { .. })
            || spec.resources.max_memory_bytes.is_some()
            || spec.resources.max_processes.is_some()
            || spec.resources.max_temp_bytes.is_some()
            || spec.resources.cpu_time_ms.is_some()
            || spec.resources.timeout_ms.is_some()
            || spec.process.max_background_lifetime_ms.is_some()
        {
            return Err(SandboxFailure::capability_unavailable(
                "Seatbelt cannot enforce allowlisting, memory, PID, temp size, CPU, or process-tree lifetime budgets",
            ));
        }
        let capabilities = self.capabilities();
        require_capabilities(spec, capabilities)?;
        let (profile, private_temp) = self.profile(spec, request)?;
        let mut environment = environment.clone();
        let home = private_temp.join("home");
        environment.insert("HOME".into(), home.to_string_lossy().into_owned());
        for name in ["TMPDIR", "TMP", "TEMP"] {
            environment.insert(name.into(), private_temp.to_string_lossy().into_owned());
        }
        // Cargo's synthetic writable home is relocated along with HOME. No
        // host credential/config files are copied into it.
        if environment
            .get("CARGO_HOME")
            .is_some_and(|value| value.starts_with("/tmp/davinci-home/"))
        {
            environment.insert(
                "CARGO_HOME".into(),
                home.join(".cargo").to_string_lossy().into_owned(),
            );
        }
        let mut argv = vec![
            "-p".into(),
            profile,
            "--".into(),
            request.executable.clone(),
        ];
        argv.extend(request.argv.iter().cloned());
        Ok(PreparedExecution {
            sandbox_id: spec.id.clone(),
            spec_digest: sandbox_spec_digest(spec)?,
            backend: self.kind(),
            capabilities,
            executable: "/usr/bin/sandbox-exec".into(),
            argv,
            cwd: PathBuf::from(&request.cwd),
            environment,
        })
    }
}

fn canonical(path: &Path) -> Result<PathBuf, SandboxFailure> {
    path.canonicalize().map_err(|error| {
        SandboxFailure::new(
            SandboxErrorCode::FilesystemDenied,
            format!("Seatbelt path unavailable: {error}"),
        )
    })
}

fn quote(path: &Path) -> Result<String, SandboxFailure> {
    let value = path
        .to_str()
        .ok_or_else(|| SandboxFailure::policy_denied("Seatbelt path is not UTF-8"))?;
    if value.chars().any(char::is_control) {
        return Err(SandboxFailure::policy_denied(
            "Seatbelt paths cannot contain control characters",
        ));
    }
    Ok(format!(
        "\"{}\"",
        value.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

fn grant(
    profile: &mut String,
    operation: &str,
    allowed: &[PathBuf],
    denied: &[PathBuf],
) -> Result<(), SandboxFailure> {
    for path in allowed {
        profile.push_str(&format!(
            "(allow {operation} (require-all (subpath {})",
            quote(path)?
        ));
        // /System/Volumes/Data is the writable host volume, including user
        // homes. A /System runtime grant must never recursively grant it.
        if path == Path::new("/System") {
            profile.push_str(" (require-not (subpath \"/System/Volumes\"))");
        }
        for denied in denied {
            profile.push_str(&format!(" (require-not (subpath {}))", quote(denied)?));
        }
        profile.push_str("))\n");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use davinci_protocol::MountRule;

    fn fixture() -> (
        tempfile::TempDir,
        tempfile::TempDir,
        SandboxSpec,
        ExecutionRequest,
    ) {
        let workspace = tempfile::tempdir().unwrap();
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("home")).unwrap();
        let path = workspace
            .path()
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let spec = SandboxSpec {
            id: SandboxId("seatbelt-test".into()),
            mode: SandboxMode::WorkspaceWrite,
            backend: SandboxBackendKind::MacosSeatbelt,
            container: None,
            workspace: path.clone(),
            filesystem: davinci_protocol::FilesystemPolicy {
                mounts: vec![MountRule {
                    source: Some(path.clone()),
                    target: path.clone(),
                    access: MountAccess::ReadWrite,
                }],
            },
            network: NetworkPolicy::Denied,
            environment: Default::default(),
            resources: Default::default(),
            process: Default::default(),
            required_capabilities: Default::default(),
        };
        let request = ExecutionRequest {
            sandbox_id: spec.id.clone(),
            executable: if cfg!(windows) {
                "C:\\Windows\\System32\\cmd.exe".into()
            } else {
                "/bin/sh".into()
            },
            argv: vec!["literal;arg".into()],
            cwd: path,
            launch_id: None,
        };
        (workspace, temp, spec, request)
    }

    #[test]
    fn profile_scopes_data_access_and_protects_absent_control_roots() {
        let (_workspace, temp, spec, request) = fixture();
        let prepared = MacosSeatbeltBackend::new(Some(temp.path()))
            .prepare(&spec, &request, &BTreeMap::new())
            .unwrap();
        assert_eq!(prepared.executable, Path::new("/usr/bin/sandbox-exec"));
        let profile = &prepared.argv[1];
        assert!(profile.contains("(deny default)"));
        for name in [".git", ".davinci", ".pi"] {
            assert!(profile.contains(&format!(
                "(require-not (subpath {}))",
                quote(&Path::new(&spec.workspace).join(name)).unwrap()
            )));
        }
        assert!(!profile.contains("(allow network"));
        assert!(!profile.contains("(subpath \"/\")"));
        assert!(profile.contains("(allow file-read-data (literal \"/\"))"));
        assert!(!profile.contains("(allow file-read* (literal \"/\"))"));
        assert!(!profile.contains("(allow file-write* (literal \"/\"))"));
        assert!(!profile.contains("(subpath \"/tmp\")"));
        assert_eq!(prepared.argv.last().unwrap(), "literal;arg");
        assert_eq!(
            prepared.environment["HOME"],
            temp.path()
                .canonicalize()
                .unwrap()
                .join("home")
                .to_string_lossy()
        );
        assert!(!prepared.capabilities.memory_limit);
        assert!(!prepared.capabilities.pid_limit);
        assert!(!prepared.capabilities.cpu_limit);
        assert!(!prepared.capabilities.ephemeral_root);
        assert!(!prepared.capabilities.ephemeral_temp);
        assert!(!prepared.capabilities.process_tree_isolation);
        assert!(!prepared.capabilities.deterministic_teardown);
        assert!(!prepared.capabilities.timeout);
    }

    #[test]
    fn mandatory_ownership_capabilities_fail_closed_before_launch() {
        let (_workspace, temp, mut spec, request) = fixture();
        let backend = MacosSeatbeltBackend::new(Some(temp.path()));
        assert!(!supports_auto_sandbox(SandboxBackendKind::MacosSeatbelt));
        assert!(supports_auto_sandbox(SandboxBackendKind::LinuxBubblewrap));
        for required in [
            SandboxCapabilities {
                process_tree_isolation: true,
                ..Default::default()
            },
            SandboxCapabilities {
                deterministic_teardown: true,
                ..Default::default()
            },
            SandboxCapabilities {
                timeout: true,
                ..Default::default()
            },
            SandboxCapabilities {
                ephemeral_temp: true,
                ..Default::default()
            },
        ] {
            spec.required_capabilities = required;
            assert_eq!(
                backend
                    .prepare(&spec, &request, &BTreeMap::new())
                    .unwrap_err()
                    .code,
                SandboxErrorCode::CapabilityUnavailable
            );
        }
    }

    #[test]
    fn system_runtime_grant_excludes_mounted_host_volumes() {
        let mut profile = String::new();
        grant(&mut profile, "file-read*", &[PathBuf::from("/System")], &[]).unwrap();
        assert!(profile.contains("(require-not (subpath \"/System/Volumes\"))"));
    }

    #[test]
    fn unsupported_resources_fail_even_without_required_bits() {
        let (_workspace, temp, mut spec, request) = fixture();
        for resource in 0..6 {
            spec.resources = Default::default();
            spec.process.max_background_lifetime_ms = None;
            match resource {
                0 => spec.resources.max_memory_bytes = Some(1024),
                1 => spec.resources.max_processes = Some(2),
                2 => spec.resources.max_temp_bytes = Some(1000),
                3 => spec.resources.cpu_time_ms = Some(1000),
                4 => spec.resources.timeout_ms = Some(1000),
                _ => spec.process.max_background_lifetime_ms = Some(1000),
            }
            assert_eq!(
                MacosSeatbeltBackend::new(Some(temp.path()))
                    .prepare(&spec, &request, &BTreeMap::new())
                    .unwrap_err()
                    .code,
                SandboxErrorCode::CapabilityUnavailable
            );
        }
    }

    #[test]
    fn profile_literals_escape_scheme_and_reject_controls() {
        assert_eq!(
            quote(Path::new("/safe/quote\"\\value")).unwrap(),
            "\"/safe/quote\\\"\\\\value\""
        );
        assert!(quote(Path::new("/safe/line\nbreak")).is_err());
    }

    #[test]
    fn hidden_mounts_and_readonly_overlays_exclude_parent_grants() {
        let (workspace, temp, mut spec, request) = fixture();
        let secret = workspace.path().join("secret");
        std::fs::create_dir(&secret).unwrap();
        let secret = secret
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        spec.filesystem.mounts.push(MountRule {
            source: None,
            target: secret.clone(),
            access: MountAccess::Hidden,
        });
        let prepared = MacosSeatbeltBackend::new(Some(temp.path()))
            .prepare(&spec, &request, &BTreeMap::new())
            .unwrap();
        assert!(prepared.argv[1].contains(&format!(
            "(require-not (subpath {}))",
            quote(Path::new(&secret)).unwrap()
        )));
    }

    #[test]
    fn arbitrary_temporary_mounts_and_relocations_fail_closed() {
        let (workspace, temp, mut spec, request) = fixture();
        spec.filesystem.mounts.push(MountRule {
            source: None,
            target: "/tmp".into(),
            access: MountAccess::Temporary,
        });
        let backend = MacosSeatbeltBackend::new(Some(temp.path()));
        assert_eq!(
            backend
                .prepare(&spec, &request, &BTreeMap::new())
                .unwrap_err()
                .code,
            SandboxErrorCode::CapabilityUnavailable
        );
        spec.filesystem.mounts.pop();
        spec.filesystem.mounts.push(MountRule {
            source: Some(spec.workspace.clone()),
            target: temp.path().to_string_lossy().into_owned(),
            access: MountAccess::ReadOnly,
        });
        assert!(backend.prepare(&spec, &request, &BTreeMap::new()).is_err());
        assert!(MacosSeatbeltBackend::new(None)
            .prepare(&spec, &request, &BTreeMap::new())
            .is_err());
        assert!(workspace.path().exists());
    }

    #[test]
    #[cfg(unix)]
    fn canonical_cwd_escape_and_protected_symlink_targets() {
        let (workspace, temp, mut spec, mut request) = fixture();
        std::os::unix::fs::symlink(temp.path(), workspace.path().join("escape")).unwrap();
        request.cwd = format!("{}/escape", spec.workspace);
        let backend = MacosSeatbeltBackend::new(Some(temp.path()));
        assert_eq!(
            backend
                .prepare(&spec, &request, &BTreeMap::new())
                .unwrap_err()
                .code,
            SandboxErrorCode::FilesystemDenied
        );
        request.cwd = spec.workspace.clone();
        let metadata = workspace.path().join("metadata");
        std::fs::create_dir(&metadata).unwrap();
        std::os::unix::fs::symlink("metadata", workspace.path().join(".git")).unwrap();
        spec.network = NetworkPolicy::Unrestricted;
        let prepared = backend.prepare(&spec, &request, &BTreeMap::new()).unwrap();
        assert!(prepared.argv[1].contains(&format!(
            "(require-not (subpath \"{}/metadata\"))",
            spec.workspace
        )));
        assert!(prepared.argv[1].contains("(allow network*)"));
    }
}
