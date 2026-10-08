//! Windows confinement: the child runs in an AppContainer.
//!
//! An AppContainer token reaches only objects whose DACL names its package
//! SID, one of its capability SIDs, or ALL APPLICATION PACKAGES. DaVinci gives
//! the token no network capability (`internetClient` and friends), so it can
//! open no socket off the machine and, without a loopback exemption, none to
//! the host either.
//!
//! Each launch uses a distinct package SID. Filesystem grants use capabilities
//! derived from mount paths, never the package SID, so another launch does not
//! acquire this one's grants unless it explicitly mounts the same path. Nested
//! mounts and hidden/protected paths fail closed: a child cannot drop a parent
//! mount's inherited capability, and temporary ACL detachment is unsafe.
//!
//! The supervisor helper already sits in a kill-on-close Job Object; the
//! launcher and its AppContainer child inherit it, so the tree dies with the
//! helper. The launcher is this executable re-entered with
//! [`LAUNCHER_ARG`]; [`run_launcher`] is its body.

use super::*;
use base64::Engine as _;
use serde::{Deserialize, Serialize};

/// The hidden argument that re-enters the executable as the launcher.
pub const LAUNCHER_ARG: &str = "--internal-appcontainer-exec";
/// The launcher's other form: the plan comes from this variable and the
/// launcher's arguments are appended to the plan's. A program that starts a
/// confined helper itself (Playwright starting the browser) names this
/// executable and sets the variable; it cannot add arguments of its own.
pub const PLAN_ENV: &str = "DAVINCI_INTERNAL_APPCONTAINER_PLAN";
/// Prefix for the per-launch AppContainer identity. Two generated pages must
/// never share the package SID or each other's browser profile and temp files.
pub const PROFILE_NAME: &str = "davinci.sandbox";

/// What the launcher needs, passed as one base64 JSON argument.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchPlan {
    pub executable: String,
    pub argv: Vec<String>,
    pub cwd: String,
    pub read: Vec<String>,
    pub write: Vec<String>,
    /// Denied entirely (read and write) for this launch.
    pub hidden: Vec<String>,
    /// Read-only for this launch even inside a writable mount.
    pub protected: Vec<String>,
}

impl LaunchPlan {
    pub fn encode(&self) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(self).expect("launch plan is JSON"))
    }

    pub fn decode(text: &str) -> Result<Self, String> {
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(text)
            .map_err(|_| "invalid AppContainer launch plan encoding")?;
        serde_json::from_slice(&bytes).map_err(|_| "invalid AppContainer launch plan".into())
    }

    /// A capability on a parent is inherited by its children. A second mount
    /// cannot take that capability away, so reject overlapping grants rather
    /// than claiming a nested read-only or hidden path is protected.
    pub fn validate(&self) -> Result<(), String> {
        if !self.hidden.is_empty() || !self.protected.is_empty() {
            return Err("AppContainer cannot safely enforce hidden or protected paths".into());
        }
        if self.read.is_empty() && self.write.is_empty() {
            return Err("AppContainer needs an explicit mount".into());
        }
        let mut roots: Vec<PathBuf> = Vec::new();
        for paths in [&self.read, &self.write] {
            for path in paths {
                let canonical = Path::new(path)
                    .canonicalize()
                    .map_err(|_| "AppContainer mount path unavailable")?;
                let root = PathBuf::from(canonical.to_string_lossy().to_lowercase());
                if roots
                    .iter()
                    .any(|other| root.starts_with(other) || other.starts_with(&root))
                {
                    return Err("AppContainer mounts must not overlap".into());
                }
                roots.push(root);
            }
        }
        Ok(())
    }
}

/// The capability name a mount's grant is keyed to: stable for a path and an
/// access level, so the grant is reused. Case-folded: Windows paths are.
pub fn mount_capability(path: &str, write: bool) -> String {
    let digest = Sha256::digest(path.to_lowercase().as_bytes());
    let hex: String = digest[..12].iter().map(|b| format!("{b:02x}")).collect();
    format!("davinciSandbox{}{hex}", if write { "W" } else { "R" })
}

#[derive(Debug, Clone)]
pub struct WindowsAppContainerBackend {
    launcher: PathBuf,
}

impl WindowsAppContainerBackend {
    pub fn new(launcher: PathBuf) -> Self {
        Self { launcher }
    }

    /// The running executable, which must answer [`LAUNCHER_ARG`].
    pub fn current() -> Option<Self> {
        std::env::current_exe().ok().map(Self::new)
    }

    fn plan(
        &self,
        spec: &SandboxSpec,
        request: &ExecutionRequest,
    ) -> Result<LaunchPlan, SandboxFailure> {
        let workspace = canonical(Path::new(&spec.workspace))?;
        if !canonical(Path::new(&request.cwd))?.starts_with(&workspace) {
            return Err(SandboxFailure::new(
                SandboxErrorCode::FilesystemDenied,
                "AppContainer cwd resolves outside workspace",
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
                "AppContainer requires an exact workspace mount",
            ));
        }
        let mut plan = LaunchPlan {
            executable: request.executable.clone(),
            argv: request.argv.clone(),
            cwd: request.cwd.clone(),
            read: Vec::new(),
            write: Vec::new(),
            hidden: Vec::new(),
            protected: Vec::new(),
        };
        for mount in &spec.filesystem.mounts {
            match mount.access {
                MountAccess::ReadOnly | MountAccess::ReadWrite => {
                    let source = canonical(Path::new(
                        mount.source.as_deref().expect("validated source"),
                    ))?;
                    if source != canonical(Path::new(&mount.target))? {
                        return Err(SandboxFailure::capability_unavailable(
                            "AppContainer cannot relocate host mounts",
                        ));
                    }
                    let path = display(&source);
                    if mount.access == MountAccess::ReadWrite {
                        plan.write.push(path);
                    } else {
                        plan.read.push(path);
                    }
                }
                MountAccess::Temporary => {
                    return Err(SandboxFailure::capability_unavailable(
                        "AppContainer provides its own private temp; temporary mounts are unsupported",
                    ));
                }
                MountAccess::Hidden => {
                    // An absent hidden path cannot be read, so there is
                    // nothing to deny; one created later is covered only if
                    // its parent is (documented limit).
                    if let Ok(path) = Path::new(&mount.target).canonicalize() {
                        plan.hidden.push(display(&path));
                    }
                }
            }
        }
        if spec.mode == SandboxMode::WorkspaceWrite {
            for name in [".git", ".davinci", ".pi"] {
                if let Ok(path) = workspace.join(name).canonicalize() {
                    plan.protected.push(display(&path));
                }
            }
        }
        for list in [
            &mut plan.read,
            &mut plan.write,
            &mut plan.hidden,
            &mut plan.protected,
        ] {
            list.sort();
            list.dedup();
        }
        Ok(plan)
    }
}

fn canonical(path: &Path) -> Result<PathBuf, SandboxFailure> {
    path.canonicalize().map_err(|error| {
        SandboxFailure::new(
            SandboxErrorCode::FilesystemDenied,
            format!("AppContainer path unavailable: {error}"),
        )
    })
}

fn display(path: &Path) -> String {
    crate::permission::strip_verbatim_prefix(path)
        .to_string_lossy()
        .into_owned()
}

impl SandboxBackend for WindowsAppContainerBackend {
    fn kind(&self) -> SandboxBackendKind {
        SandboxBackendKind::WindowsAppContainer
    }

    fn capabilities(&self) -> SandboxCapabilities {
        SandboxCapabilities {
            filesystem_isolation: true,
            network_denied: true,
            environment_isolation: true,
            // The helper's kill-on-close job owns the launcher and the child.
            process_tree_isolation: true,
            ephemeral_temp: true,
            output_limit: true,
            timeout: true,
            deterministic_teardown: true,
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
        if !cfg!(windows) {
            return Err(SandboxFailure::new(
                SandboxErrorCode::SandboxUnavailable,
                "AppContainer requires native Windows",
            ));
        }
        if spec.mode != SandboxMode::Restricted {
            return Err(SandboxFailure::capability_unavailable(
                "AppContainer cannot safely enforce workspace-write protected paths",
            ));
        }
        if spec
            .filesystem
            .mounts
            .iter()
            .any(|mount| mount.access == MountAccess::Hidden)
        {
            return Err(SandboxFailure::capability_unavailable(
                "AppContainer cannot safely enforce hidden paths inside a readable mount",
            ));
        }
        if !matches!(spec.network, NetworkPolicy::Denied) {
            return Err(SandboxFailure::capability_unavailable(
                "AppContainer enforces denied networking only",
            ));
        }
        if spec.resources.max_memory_bytes.is_some()
            || spec.resources.max_processes.is_some()
            || spec.resources.max_temp_bytes.is_some()
            || spec.resources.cpu_time_ms.is_some()
        {
            return Err(SandboxFailure::capability_unavailable(
                "AppContainer does not enforce memory, PID, temp size or CPU budgets",
            ));
        }
        let capabilities = self.capabilities();
        require_capabilities(spec, capabilities)?;
        let plan = self.plan(spec, request)?;
        Ok(PreparedExecution {
            sandbox_id: spec.id.clone(),
            spec_digest: sandbox_spec_digest(spec)?,
            backend: self.kind(),
            capabilities,
            executable: self.launcher.clone(),
            argv: vec![LAUNCHER_ARG.into(), plan.encode()],
            cwd: PathBuf::from(&request.cwd),
            environment: environment.clone(),
        })
    }
}

/// The launcher: set up grants, start the child in the AppContainer with the
/// launcher's own stdio and environment, wait, and return its exit code.
#[cfg(windows)]
pub fn run_launcher(encoded: &str) -> i32 {
    run_plan(encoded, &[])
}

/// [`PLAN_ENV`]'s form: `args` are appended to the plan's arguments. The
/// variable is removed before the child starts.
#[cfg(windows)]
pub fn run_env_launcher(args: &[String]) -> i32 {
    let Ok(encoded) = std::env::var(PLAN_ENV) else {
        eprintln!("davinci sandbox: no launch plan");
        return 125;
    };
    std::env::remove_var(PLAN_ENV);
    run_plan(&encoded, args)
}

#[cfg(windows)]
fn run_plan(encoded: &str, extra: &[String]) -> i32 {
    let plan = match LaunchPlan::decode(encoded) {
        Ok(mut plan) => {
            plan.argv.extend(extra.iter().cloned());
            plan
        }
        Err(error) => {
            eprintln!("davinci sandbox: {error}");
            return 125;
        }
    };
    match native::launch(plan) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("davinci sandbox: {error}");
            // The supervisor reports a failure before the child ran.
            125
        }
    }
}

#[cfg(not(windows))]
pub fn run_launcher(_encoded: &str) -> i32 {
    eprintln!("davinci sandbox: AppContainer requires native Windows");
    125
}

#[cfg(not(windows))]
pub fn run_env_launcher(_args: &[String]) -> i32 {
    run_launcher("")
}

/// What a running process's token says about its confinement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessConfinement {
    pub app_container: bool,
    /// Holds `internetClient`, `internetClientServer` or
    /// `privateNetworkClientServer`.
    pub network_capability: bool,
    /// The token's mandatory integrity RID (0x1000 Low, 0x2000 Medium).
    pub integrity: u32,
}

/// Read `pid`'s token. `None` when the process is gone or unreadable.
#[cfg(windows)]
pub fn process_confinement(pid: u32) -> Option<ProcessConfinement> {
    native::process_confinement(pid)
}

#[cfg(not(windows))]
pub fn process_confinement(_pid: u32) -> Option<ProcessConfinement> {
    None
}

#[cfg(windows)]
mod native {
    use super::{mount_capability, LaunchPlan, PROFILE_NAME};
    use std::ffi::{c_void, OsStr};
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, LocalFree, SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT,
        INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
    };
    use windows_sys::Win32::Security::Authorization::{
        GetNamedSecurityInfoW, SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W,
        GRANT_ACCESS, NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT, TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN,
        TRUSTEE_W,
    };
    use windows_sys::Win32::Security::Isolation::{
        CreateAppContainerProfile, DeleteAppContainerProfile, GetAppContainerFolderPath,
    };
    use windows_sys::Win32::Security::{
        DeriveCapabilitySidsFromName, EqualSid, FreeSid, GetAce, ACCESS_ALLOWED_ACE, ACE_HEADER,
        ACL, DACL_SECURITY_INFORMATION, NO_INHERITANCE, PSID, SECURITY_CAPABILITIES,
        SID_AND_ATTRIBUTES, SUB_CONTAINERS_AND_OBJECTS_INHERIT,
    };
    use windows_sys::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess, GetStartupInfoW,
        InitializeProcThreadAttributeList, ResumeThread, UpdateProcThreadAttribute,
        WaitForSingleObject, CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT,
        EXTENDED_STARTUPINFO_PRESENT, INFINITE, LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION,
        PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
        STARTF_USESTDHANDLES, STARTUPINFOEXW, STARTUPINFOW,
    };

    const FILE_READ_EXECUTE: u32 = 0x0012_00A9;
    // Data, attributes, delete, read-control and synchronize; not WRITE_DAC
    // or WRITE_OWNER. Writable mounts are private temp, not host workspaces.
    const FILE_TEMP_ACCESS: u32 = 0x0013_01FF;
    const SE_GROUP_ENABLED: u32 = 0x4;
    const INHERIT_ONLY_ACE: u8 = 0x08;
    const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
    /// "ALL APPLICATION PACKAGES"
    const ALL_APP_PACKAGES: &str = "S-1-15-2-1";

    fn wide(text: impl AsRef<OsStr>) -> Vec<u16> {
        text.as_ref().encode_wide().chain(Some(0)).collect()
    }

    fn last_error(what: &str) -> String {
        format!("{what} failed ({})", unsafe { GetLastError() })
    }

    /// A SID allocated by the OS, freed with the matching call.
    struct Sid {
        sid: PSID,
        free: fn(PSID),
    }

    impl Drop for Sid {
        fn drop(&mut self) {
            if !self.sid.is_null() {
                (self.free)(self.sid);
            }
        }
    }

    fn free_sid(sid: PSID) {
        unsafe {
            FreeSid(sid);
        }
    }

    fn local_free(sid: PSID) {
        unsafe {
            LocalFree(sid as _);
        }
    }

    struct Package {
        name: Vec<u16>,
        sid: Sid,
    }

    impl Drop for Package {
        fn drop(&mut self) {
            // The child job must close before this profile is removed.
            let _ = unsafe { DeleteAppContainerProfile(self.name.as_ptr()) };
        }
    }

    fn package_sid() -> Result<Package, String> {
        let name = wide(format!("{PROFILE_NAME}.{}", uuid::Uuid::new_v4().simple()));
        let mut sid: PSID = null_mut();
        let created = unsafe {
            CreateAppContainerProfile(
                name.as_ptr(),
                name.as_ptr(),
                wide("DaVinci sandboxed tools").as_ptr(),
                null(),
                0,
                &mut sid,
            )
        };
        if created != 0 {
            return Err(format!("CreateAppContainerProfile failed ({created:#x})"));
        }
        Ok(Package {
            name,
            sid: Sid {
                sid,
                free: free_sid,
            },
        })
    }

    fn capability_sid(name: &str) -> Result<Sid, String> {
        let name = wide(name);
        let (mut groups, mut group_count, mut caps, mut cap_count) =
            (null_mut::<PSID>(), 0u32, null_mut::<PSID>(), 0u32);
        let ok = unsafe {
            DeriveCapabilitySidsFromName(
                name.as_ptr(),
                &mut groups,
                &mut group_count,
                &mut caps,
                &mut cap_count,
            )
        };
        if ok == 0 {
            return Err(last_error("DeriveCapabilitySidsFromName"));
        }
        unsafe {
            for index in 0..group_count as usize {
                LocalFree(*groups.add(index) as _);
            }
            LocalFree(groups as _);
            let sid = if cap_count > 0 { *caps } else { null_mut() };
            for index in 1..cap_count as usize {
                LocalFree(*caps.add(index) as _);
            }
            LocalFree(caps as _);
            if sid.is_null() {
                return Err("capability SID unavailable".into());
            }
            Ok(Sid {
                sid,
                free: local_free,
            })
        }
    }

    fn string_sid(text: &str) -> Result<Sid, String> {
        let mut sid: PSID = null_mut();
        let ok = unsafe {
            windows_sys::Win32::Security::Authorization::ConvertStringSidToSidW(
                wide(text).as_ptr(),
                &mut sid,
            )
        };
        if ok == 0 {
            return Err(last_error("ConvertStringSidToSidW"));
        }
        Ok(Sid {
            sid,
            free: local_free,
        })
    }

    /// The DACL of `path` and the descriptor that owns it.
    struct Dacl {
        acl: *mut ACL,
        descriptor: *mut c_void,
    }

    impl Drop for Dacl {
        fn drop(&mut self) {
            unsafe {
                LocalFree(self.descriptor as _);
            }
        }
    }

    fn dacl(path: &[u16]) -> Result<Dacl, String> {
        let (mut acl, mut descriptor) = (null_mut::<ACL>(), null_mut::<c_void>());
        let status = unsafe {
            GetNamedSecurityInfoW(
                path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                &mut acl,
                null_mut(),
                &mut descriptor,
            )
        };
        if status != 0 {
            return Err(format!("reading the ACL failed ({status})"));
        }
        Ok(Dacl { acl, descriptor })
    }

    /// Whether an effective allow entry for one of `sids` covers `mask`.
    fn allows(dacl: &Dacl, sids: &[&Sid], mask: u32) -> bool {
        if dacl.acl.is_null() {
            // A NULL DACL grants everything.
            return true;
        }
        let count = unsafe { (*dacl.acl).AceCount };
        (0..u32::from(count)).any(|index| {
            let mut ace: *mut c_void = null_mut();
            if unsafe { GetAce(dacl.acl, index, &mut ace) } == 0 {
                return false;
            }
            let header = unsafe { &*(ace as *const ACE_HEADER) };
            if header.AceType != ACCESS_ALLOWED_ACE_TYPE || header.AceFlags & INHERIT_ONLY_ACE != 0
            {
                return false;
            }
            let allowed = unsafe { &*(ace as *const ACCESS_ALLOWED_ACE) };
            let sid = (&allowed.SidStart as *const u32) as PSID;
            allowed.Mask & mask == mask
                && sids
                    .iter()
                    .any(|candidate| unsafe { EqualSid(sid, candidate.sid) } != 0)
        })
    }

    fn apply(path: &[u16], sid: &Sid, mode: i32, mask: u32, directory: bool) -> Result<(), String> {
        let current = dacl(path)?;
        let entry = EXPLICIT_ACCESS_W {
            grfAccessPermissions: mask,
            grfAccessMode: mode,
            grfInheritance: if directory {
                SUB_CONTAINERS_AND_OBJECTS_INHERIT
            } else {
                NO_INHERITANCE
            },
            Trustee: TRUSTEE_W {
                pMultipleTrustee: null_mut(),
                MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_UNKNOWN,
                ptstrName: sid.sid as _,
            },
        };
        let mut updated: *mut ACL = null_mut();
        let status = unsafe { SetEntriesInAclW(1, &entry, current.acl, &mut updated) };
        if status != 0 {
            return Err(format!("building the ACL failed ({status})"));
        }
        let status = unsafe {
            SetNamedSecurityInfoW(
                path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                updated,
                null(),
            )
        };
        unsafe {
            LocalFree(updated as _);
        }
        if status != 0 {
            return Err(format!("writing the ACL failed ({status})"));
        }
        Ok(())
    }

    /// A read mount is not read-only if Windows already grants ALL APPLICATION
    /// PACKAGES write/delete on a descendant. Check the existing tree before
    /// adding any capability ACLs; a nested explicit ACE cannot be overridden
    /// by a grant on its parent.
    fn check_read_mount(path: &str, everyone: &Sid) -> Result<(), String> {
        const WRITE_RIGHTS: &[u32] = &[
            0x2,
            0x4,
            0x10,
            0x40,
            0x100,
            0x1_0000,
            0x4_0000,
            0x8_0000,
            0x1000_0000,
            0x4000_0000,
        ];
        let mut pending = vec![std::path::PathBuf::from(path)];
        let mut examined = 0usize;
        while let Some(current) = pending.pop() {
            examined += 1;
            if examined > 10_000 {
                return Err("AppContainer read mount has too many entries to verify".into());
            }
            let metadata = std::fs::symlink_metadata(&current)
                .map_err(|error| format!("inspecting read mount: {error}"))?;
            if metadata.file_type().is_symlink() {
                return Err("AppContainer read mount contains a link".into());
            }
            let acl = dacl(&wide(current.as_os_str()))?;
            if WRITE_RIGHTS
                .iter()
                .any(|right| allows(&acl, &[everyone], *right))
            {
                return Err(
                    "AppContainer read mount is writable by all application packages".into(),
                );
            }
            if metadata.is_dir() {
                for child in std::fs::read_dir(&current)
                    .map_err(|error| format!("enumerating read mount: {error}"))?
                {
                    pending.push(child.map_err(|error| error.to_string())?.path());
                }
            }
        }
        Ok(())
    }

    /// Grant `mask` on `path` to `cap` unless an entry already covers it.
    fn grant(path: &str, cap: &Sid, everyone: &Sid, mask: u32) -> Result<(), String> {
        let wide_path = wide(path);
        if allows(&dacl(&wide_path)?, &[cap, everyone], mask) {
            return Ok(());
        }
        let directory = std::path::Path::new(path).is_dir();
        apply(&wide_path, cap, GRANT_ACCESS, mask, directory)
            .map_err(|error| format!("granting sandbox access to {path}: {error}"))
    }

    fn quote(argument: &str) -> String {
        // CommandLineToArgvW rules, as std::process uses.
        if !argument.is_empty() && !argument.contains([' ', '\t', '\n', '\u{b}', '"']) {
            return argument.to_string();
        }
        let mut quoted = String::from("\"");
        let mut backslashes = 0;
        for character in argument.chars() {
            if character == '\\' {
                backslashes += 1;
                continue;
            }
            if character == '"' {
                quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
            } else {
                quoted.push_str(&"\\".repeat(backslashes));
            }
            backslashes = 0;
            quoted.push(character);
        }
        quoted.push_str(&"\\".repeat(backslashes * 2));
        quoted.push('"');
        quoted
    }

    fn container_folder(package: &Sid) -> Result<std::path::PathBuf, String> {
        let mut text: *mut u16 = null_mut();
        if unsafe {
            windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW(
                package.sid,
                &mut text,
            )
        } == 0
        {
            return Err(last_error("ConvertSidToStringSidW"));
        }
        let mut folder: *mut u16 = null_mut();
        let status = unsafe { GetAppContainerFolderPath(text, &mut folder) };
        unsafe {
            LocalFree(text as _);
        }
        if status != 0 {
            return Err(format!("GetAppContainerFolderPath failed ({status:#x})"));
        }
        let length = (0..)
            .take_while(|&i| unsafe { *folder.add(i) } != 0)
            .count();
        let path = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(folder, length) });
        unsafe {
            windows_sys::Win32::System::Com::CoTaskMemFree(folder as _);
        }
        Ok(std::path::PathBuf::from(path))
    }

    /// The handles a parent passed in the C runtime's descriptor table
    /// (`lpReserved2`: a count, one flag byte per descriptor, then one handle
    /// per descriptor). libuv passes descriptors beyond stdio this way, as
    /// Playwright's `--remote-debugging-pipe` (descriptors 3 and 4) needs.
    fn crt_handles(startup: &STARTUPINFOW) -> Vec<HANDLE> {
        let size = usize::from(startup.cbReserved2);
        if startup.lpReserved2.is_null() || size < 4 {
            return Vec::new();
        }
        let bytes = unsafe { std::slice::from_raw_parts(startup.lpReserved2, size) };
        let count = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        let width = std::mem::size_of::<HANDLE>();
        if count == 0 || size < 4 + count + count * width {
            return Vec::new();
        }
        (0..count)
            .filter_map(|index| {
                let start = 4 + count + index * width;
                let mut raw = [0u8; 8];
                raw[..width].copy_from_slice(&bytes[start..start + width]);
                let handle = usize::from_le_bytes(raw) as HANDLE;
                (!handle.is_null() && handle != INVALID_HANDLE_VALUE && handle as isize != -2)
                    .then_some(handle)
            })
            .collect()
    }

    /// A job that kills the child's tree when this launcher dies, however it
    /// dies. Nested inside the supervisor's job when there is one.
    fn kill_on_close_job() -> Result<HANDLE, String> {
        let job = unsafe { CreateJobObjectW(null(), null()) };
        if job.is_null() {
            return Err(last_error("CreateJobObjectW"));
        }
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        } == 0
        {
            return Err(last_error("SetInformationJobObject"));
        }
        Ok(job)
    }

    pub(super) fn launch(plan: LaunchPlan) -> Result<i32, String> {
        plan.validate()?;
        let everyone = string_sid(ALL_APP_PACKAGES)?;
        for path in &plan.read {
            check_read_mount(path, &everyone)?;
        }
        let package = package_sid()?;
        let mut capabilities = Vec::new();
        for (paths, write) in [(&plan.read, false), (&plan.write, true)] {
            for path in paths {
                let cap = capability_sid(&mount_capability(path, write))?;
                let mask = if write {
                    FILE_TEMP_ACCESS
                } else {
                    FILE_READ_EXECUTE
                };
                grant(path, &cap, &everyone, mask)?;
                capabilities.push(cap);
            }
        }
        // The container's own folder holds this launch's temp and profile.
        // Windows rebuilds an AppContainer child's TEMP and TMP from its
        // LOCALAPPDATA (`<LOCALAPPDATA>\Packages\<name>\AC\Temp`), so the
        // private LOCALAPPDATA gets that layout and TEMP lands inside it.
        let private = container_folder(&package.sid)?
            .join("Temp")
            .join(uuid::Uuid::new_v4().simple().to_string());
        let home = private.join("home");
        let local = private.join("local");
        let temp = local
            .join("Packages")
            .join(String::from_utf16_lossy(
                &package.name[..package.name.len() - 1],
            ))
            .join("AC")
            .join("Temp");
        for directory in [&home, &temp, &private.join("roaming")] {
            std::fs::create_dir_all(directory).map_err(|error| format!("private temp: {error}"))?;
        }
        let _cleanup = RemoveOnDrop(private.clone());
        for name in ["TEMP", "TMP", "TMPDIR"] {
            std::env::set_var(name, &temp);
        }
        for name in ["USERPROFILE", "HOME"] {
            std::env::set_var(name, &home);
        }
        std::env::set_var("LOCALAPPDATA", &local);
        std::env::set_var("APPDATA", private.join("roaming"));

        let mut attributes: Vec<SID_AND_ATTRIBUTES> = capabilities
            .iter()
            .map(|cap| SID_AND_ATTRIBUTES {
                Sid: cap.sid,
                Attributes: SE_GROUP_ENABLED,
            })
            .collect();
        let security = SECURITY_CAPABILITIES {
            AppContainerSid: package.sid.sid,
            Capabilities: attributes.as_mut_ptr(),
            CapabilityCount: attributes.len() as u32,
            Reserved: 0,
        };

        let mut own: STARTUPINFOW = unsafe { std::mem::zeroed() };
        own.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        unsafe { GetStartupInfoW(&mut own) };
        let mut handles: Vec<HANDLE> = Vec::new();
        let stdio = [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE]
            .map(|kind| unsafe { GetStdHandle(kind) });
        for handle in stdio.into_iter().chain(crt_handles(&own)) {
            if handle.is_null() || handle == INVALID_HANDLE_VALUE || handles.contains(&handle) {
                continue;
            }
            if unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) }
                == 0
            {
                return Err(last_error("SetHandleInformation"));
            }
            handles.push(handle);
        }
        if handles.is_empty() {
            return Err("launcher stdio unavailable".into());
        }

        let mut size = 0usize;
        unsafe {
            InitializeProcThreadAttributeList(null_mut(), 2, 0, &mut size);
        }
        let mut buffer = vec![0u8; size];
        let list = buffer.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
        if unsafe { InitializeProcThreadAttributeList(list, 2, 0, &mut size) } == 0 {
            return Err(last_error("InitializeProcThreadAttributeList"));
        }
        struct List(LPPROC_THREAD_ATTRIBUTE_LIST);
        impl Drop for List {
            fn drop(&mut self) {
                unsafe { DeleteProcThreadAttributeList(self.0) }
            }
        }
        let _list = List(list);
        if unsafe {
            UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
                (&security as *const SECURITY_CAPABILITIES).cast(),
                std::mem::size_of::<SECURITY_CAPABILITIES>(),
                null_mut(),
                null(),
            )
        } == 0
        {
            return Err(last_error("UpdateProcThreadAttribute(security)"));
        }
        // Only the three stdio handles are inherited, nothing else open here.
        if unsafe {
            UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                handles.as_ptr().cast(),
                std::mem::size_of_val(handles.as_slice()),
                null_mut(),
                null(),
            )
        } == 0
        {
            return Err(last_error("UpdateProcThreadAttribute(handles)"));
        }

        let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
        startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = stdio[0];
        startup.StartupInfo.hStdOutput = stdio[1];
        startup.StartupInfo.hStdError = stdio[2];
        // The same descriptor table, naming the same inherited handles.
        startup.StartupInfo.cbReserved2 = own.cbReserved2;
        startup.StartupInfo.lpReserved2 = own.lpReserved2;
        startup.lpAttributeList = list;
        struct Job(HANDLE);
        impl Drop for Job {
            fn drop(&mut self) {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
        let job = Job(kill_on_close_job()?);

        let mut command_line: Vec<u16> = wide(
            std::iter::once(plan.executable.as_str())
                .chain(plan.argv.iter().map(String::as_str))
                .map(quote)
                .collect::<Vec<_>>()
                .join(" "),
        );
        let executable = wide(&plan.executable);
        let cwd = wide(&plan.cwd);
        let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        let created = unsafe {
            CreateProcessW(
                executable.as_ptr(),
                command_line.as_mut_ptr(),
                null(),
                null(),
                1,
                EXTENDED_STARTUPINFO_PRESENT
                    | CREATE_UNICODE_ENVIRONMENT
                    | CREATE_NO_WINDOW
                    | CREATE_SUSPENDED,
                null(),
                cwd.as_ptr(),
                &startup.StartupInfo,
                &mut process,
            )
        };
        if created == 0 {
            return Err(last_error("starting the sandboxed process"));
        }
        // Into the job before it runs a single instruction.
        if unsafe { AssignProcessToJobObject(job.0, process.hProcess) } == 0 {
            let error = last_error("AssignProcessToJobObject");
            unsafe {
                windows_sys::Win32::System::Threading::TerminateProcess(process.hProcess, 1);
                CloseHandle(process.hThread);
                CloseHandle(process.hProcess);
            }
            return Err(error);
        }
        unsafe {
            ResumeThread(process.hThread);
            CloseHandle(process.hThread);
        }
        let waited = unsafe { WaitForSingleObject(process.hProcess, INFINITE) };
        let mut code = 1u32;
        let read = unsafe { GetExitCodeProcess(process.hProcess, &mut code) };
        unsafe {
            CloseHandle(process.hProcess);
        }
        if waited != WAIT_OBJECT_0 || read == 0 {
            return Err("waiting for the sandboxed process failed".into());
        }
        Ok(code as i32)
    }

    pub(super) fn process_confinement(pid: u32) -> Option<super::ProcessConfinement> {
        use windows_sys::Win32::Security::{
            GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenCapabilities,
            TokenIntegrityLevel, TokenIsAppContainer, TOKEN_GROUPS, TOKEN_MANDATORY_LABEL,
            TOKEN_QUERY,
        };
        use windows_sys::Win32::System::Threading::{
            OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        struct Owned(HANDLE);
        impl Drop for Owned {
            fn drop(&mut self) {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if process.is_null() {
            return None;
        }
        let process = Owned(process);
        let mut token: HANDLE = null_mut();
        if unsafe { OpenProcessToken(process.0, TOKEN_QUERY, &mut token) } == 0 {
            return None;
        }
        let token = Owned(token);
        let mut flag = 0u32;
        let mut size = 0u32;
        if unsafe {
            GetTokenInformation(
                token.0,
                TokenIsAppContainer,
                (&mut flag as *mut u32).cast(),
                4,
                &mut size,
            )
        } == 0
        {
            return None;
        }
        let mut needed = 0u32;
        unsafe {
            GetTokenInformation(token.0, TokenCapabilities, null_mut(), 0, &mut needed);
        }
        let mut network_capability = false;
        if needed > 0 {
            // u64 storage keeps TOKEN_GROUPS suitably aligned.
            let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
            if unsafe {
                GetTokenInformation(
                    token.0,
                    TokenCapabilities,
                    buffer.as_mut_ptr().cast(),
                    needed,
                    &mut needed,
                )
            } == 0
            {
                return None;
            }
            let groups = unsafe { &*(buffer.as_ptr() as *const TOKEN_GROUPS) };
            let entries = unsafe {
                std::slice::from_raw_parts(groups.Groups.as_ptr(), groups.GroupCount as usize)
            };
            // internetClient, internetClientServer, privateNetworkClientServer
            let network: Vec<Sid> = ["S-1-15-3-1", "S-1-15-3-2", "S-1-15-3-3"]
                .iter()
                .filter_map(|text| string_sid(text).ok())
                .collect();
            network_capability = entries.iter().any(|entry| {
                network
                    .iter()
                    .any(|sid| unsafe { EqualSid(entry.Sid, sid.sid) } != 0)
            });
        }
        let mut integrity = 0u32;
        let mut needed = 0u32;
        unsafe {
            GetTokenInformation(token.0, TokenIntegrityLevel, null_mut(), 0, &mut needed);
        }
        if needed > 0 {
            let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
            if unsafe {
                GetTokenInformation(
                    token.0,
                    TokenIntegrityLevel,
                    buffer.as_mut_ptr().cast(),
                    needed,
                    &mut needed,
                )
            } != 0
            {
                let label = unsafe { &*(buffer.as_ptr() as *const TOKEN_MANDATORY_LABEL) };
                unsafe {
                    let count = *GetSidSubAuthorityCount(label.Label.Sid);
                    if count > 0 {
                        integrity = *GetSidSubAuthority(label.Label.Sid, u32::from(count) - 1);
                    }
                }
            }
        }
        Some(super::ProcessConfinement {
            app_container: flag != 0,
            network_capability,
            integrity,
        })
    }

    struct RemoveOnDrop(std::path::PathBuf);

    impl Drop for RemoveOnDrop {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_launch_plan_survives_its_argument_encoding() {
        let plan = LaunchPlan {
            executable: r"C:\Program Files\nodejs\node.exe".into(),
            argv: vec!["host.js".into(), "a \"quoted\" arg".into()],
            cwd: r"C:\work\private".into(),
            read: vec![r"C:\runtime".into()],
            write: vec![r"C:\work\private".into()],
            hidden: vec![],
            protected: vec![r"C:\work\private\.git".into()],
        };
        let encoded = plan.encode();
        assert!(!encoded.contains([' ', '"', '\\']));
        assert_eq!(LaunchPlan::decode(&encoded).unwrap(), plan);
        assert!(LaunchPlan::decode("not base64!").is_err());
    }

    #[test]
    fn mount_capabilities_are_stable_per_path_and_access() {
        let read = mount_capability(r"C:\Runtime\Node", false);
        assert_eq!(read, mount_capability(r"c:\runtime\node", false));
        assert_ne!(read, mount_capability(r"C:\Runtime\Node", true));
        assert_ne!(read, mount_capability(r"C:\Runtime\Other", false));
        assert!(read.starts_with("davinciSandboxR") && read.len() == 15 + 24);
    }
}
