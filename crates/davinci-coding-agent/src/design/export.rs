//! Explicit local exports; generated HTML is never opened or deployed here.
use super::{
    admission::*, commands::ExportFormat, error::*, records::*, runtime::*, skills::byte_hash,
    store::*, types::*,
};
use davinci_session::JsonlSession;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
};

struct Staging(PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        // Only the freshly created, private export staging directory is owned here.
        if self.0.is_dir() && no_links(&self.0).is_ok() {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportRequest {
    pub artifact_id: ArtifactId,
    pub revision: RevisionId,
    pub format: ExportFormat,
    pub destination: String,
    pub artboard_id: Option<ArtboardId>,
    pub viewport: Option<Viewport>,
    pub operation_id: OperationId,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportReceipt {
    pub artifact_id: ArtifactId,
    pub revision: RevisionId,
    pub source_hash: String,
    pub manifest_hash: String,
    pub paths: Vec<String>,
    pub omitted_assets: Vec<String>,
    pub complete: bool,
    pub warning: Option<String>,
}
fn write_new(root: &Path, relative: &str, data: &[u8]) -> DesignResult<()> {
    validate_path(relative)?;
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        no_links(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(data)?;
    file.sync_all()?;
    Ok(())
}
fn publish_new(source: &Path, destination: &Path) -> DesignResult<()> {
    #[cfg(windows)]
    {
        fs::rename(source, destination)?;
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let source = CString::new(source.as_os_str().as_bytes())
            .map_err(|_| DesignError::InvalidInput("export path NUL".into()))?;
        let destination = CString::new(destination.as_os_str().as_bytes())
            .map_err(|_| DesignError::InvalidInput("export path NUL".into()))?;
        // SAFETY: valid NUL-terminated paths; no-replace is an atomic kernel operation.
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                libc::AT_FDCWD,
                source.as_ptr(),
                libc::AT_FDCWD,
                destination.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        #[cfg(target_os = "macos")]
        let result =
            unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) };
        if result != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        return Err(DesignError::MissingCapability(
            "atomic no-replace export is unavailable on this platform".into(),
        ));
    }
    Ok(())
}
pub fn export_revision(
    store: &DesignStore,
    ctx: &AuthorizedDesignContext,
    session: &JsonlSession,
    request: &ExportRequest,
    runtime: Option<&TrustedDesignRuntime>,
) -> DesignResult<ExportReceipt> {
    let args = serde_json::to_value(request)?;
    ctx.check("design_export", &args)?;
    let revision = store.read_revision(ctx, session, request.artifact_id, request.revision)?;
    let requested = Path::new(&request.destination);
    if !requested.is_absolute()
        || request
            .destination
            .split(['/', '\\'])
            .any(|part| part == "." || part == "..")
        || requested
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(DesignError::InvalidInput(
            "select an absolute new destination directory without traversal".into(),
        ));
    }
    let parent = requested
        .parent()
        .ok_or_else(|| DesignError::InvalidInput("export parent missing".into()))?;
    no_links(parent)?;
    let parent = parent.canonicalize()?;
    let name = requested
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| DesignError::InvalidInput("export name missing".into()))?;
    validate_path(name)?;
    let destination = parent.join(name);
    if destination.try_exists()? || fs::symlink_metadata(&destination).is_ok() {
        return Err(DesignError::Conflict(
            "export destination already exists; choose a new directory".into(),
        ));
    }
    let mut files = BTreeMap::<String, Vec<u8>>::new();
    let mut omitted = Vec::new();
    let mut warning = None;
    let sources = store.read_sources(ctx, session, request.artifact_id, request.revision)?;
    match request.format {
        ExportFormat::Source => {
            for (name, text) in &sources {
                files.insert(format!("source/{name}"), text.as_bytes().to_vec());
            }
        }
        ExportFormat::Png => {
            let runtime = runtime.ok_or_else(|| {
                DesignError::MissingCapability(
                    "PNG export requires the pinned runtime to validate capture fingerprints"
                        .into(),
                )
            })?;
            runtime.verify()?;
            let receipts = store.receipts(ctx, session, request.artifact_id, request.revision)?;
            let receipt = receipts
                .iter()
                .find(|r| {
                    Some(r.request.artboard_id) == request.artboard_id
                        && Some(&r.request.viewport) == request.viewport.as_ref()
                })
                .ok_or_else(|| {
                    DesignError::IncompleteEvidence("requested capture is missing".into())
                })?;
            if receipt.source_hash != revision.source_hash
                || receipt.runtime_hash != runtime.fingerprint()
                || receipt.policy_hash != super::render::policy_hash()
            {
                return Err(DesignError::StaleSource(
                    "capture does not match source/runtime/policy".into(),
                ));
            }
            files.insert(
                "capture.png".into(),
                store.read_blob(ctx, &receipt.screenshot)?,
            );
        }
        ExportFormat::Html => {
            let boards: Vec<_> = revision
                .variants
                .iter()
                .flat_map(|v| &v.artboards)
                .collect();
            let board = request
                .artboard_id
                .and_then(|id| boards.iter().find(|b| b.id == id).copied())
                .or_else(|| (boards.len() == 1).then(|| boards[0]))
                .ok_or_else(|| {
                    DesignError::InvalidInput(
                        "HTML export requires one explicitly selected artboard".into(),
                    )
                })?;
            let runtime = runtime.ok_or_else(|| {
                DesignError::MissingCapability("HTML export requires the pinned compiler".into())
            })?;
            let compiled = super::compile::compile(ctx, runtime, &board.entry_point, &sources)?;
            for (name, text) in compiled.files {
                files.insert(format!("html/{name}"), text.into_bytes());
            }
            warning = Some("Executable HTML. Opening outside DaVinci relinquishes preview isolation. External requests and scripts may execute. This package has not been opened, uploaded or deployed.".into());
        }
    }
    for asset in &revision.assets {
        // Unknown rights, including unlicensed fonts, are never silently redistributed.
        if !matches!(
            asset.rights.as_str(),
            "CC0-1.0" | "user-owned; redistribution permitted"
        ) {
            omitted.push(asset.path.clone());
            continue;
        }
        let name = match request.format {
            ExportFormat::Source => format!("source/{}", asset.path),
            ExportFormat::Html => format!("html/{}", asset.path),
            ExportFormat::Png => continue,
        };
        if files
            .insert(name, store.read_blob(ctx, &asset.source)?)
            .is_some()
        {
            return Err(DesignError::Conflict(
                "asset collides with an exported source file".into(),
            ));
        }
    }
    let manifest = serde_json::json!({"schema_version":1,"artifact_id":revision.artifact_id,"revision":revision.revision,
        "source_hash":revision.source_hash,"sources":revision.sources,"variants":revision.variants,"bindings":revision.bindings,
        "assets":revision.assets.iter().filter(|a| !omitted.contains(&a.path)).collect::<Vec<_>>(),
        "omitted_assets":omitted,"runtime_hash":runtime.map(|r|r.fingerprint()),"policy_hash":super::render::policy_hash(),
        "files":files.iter().map(|(p,b)|(p.clone(),byte_hash(b))).collect::<BTreeMap<_,_>>()});
    let encoded = serde_json::to_vec_pretty(&manifest)?;
    let manifest_hash = byte_hash(&encoded);
    files.insert("design.json".into(), encoded);
    files.insert("README.txt".into(), format!("DaVinci design revision {}\nSource SHA-256: {}\n\nSource files retain their exact bytes. design.json maps entrypoints and content hashes; private session metadata and prompts are excluded. This is a preview, not a verified production integration. React 19.3.0, esbuild 0.25.11; see toolchain.json and LICENSES.txt.\nOmitted assets: {:?}. Missing required assets make this package incomplete.\n{}\n", revision.revision, revision.source_hash, omitted, warning.as_deref().unwrap_or("No output was automatically executed.")).into_bytes());
    files.insert(
        "toolchain.json".into(),
        include_bytes!("../../design-resources/toolchain-manifest.json").to_vec(),
    );
    files.insert(
        "LICENSES.txt".into(),
        include_bytes!("../../design-resources/runtime-LICENSES.txt").to_vec(),
    );
    let total: usize = files.values().map(Vec::len).sum();
    if total > 100 * 1024 * 1024 {
        return Err(DesignError::BudgetExceeded("export byte budget".into()));
    }
    let staging = Staging(parent.join(format!(".davinci-design-export-{}", uuid::Uuid::new_v4())));
    fs::create_dir(&staging.0)?;
    for (name, bytes) in &files {
        ctx.check("design_export", &args)?;
        write_new(&staging.0, name, bytes)?;
    }
    ctx.check("design_export", &args)?;
    ctx.check_session(session)?;
    no_links(&parent)?;
    no_links(&staging.0)?;
    publish_new(&staging.0, &destination)?;
    davinci_sys::fs::sync_parent(&parent)?;
    Ok(ExportReceipt {
        artifact_id: revision.artifact_id,
        revision: revision.revision,
        source_hash: revision.source_hash,
        manifest_hash,
        paths: files.keys().cloned().collect(),
        complete: omitted.is_empty(),
        omitted_assets: omitted,
        warning,
    })
}
