//! Explicitly installed and fingerprinted tooling, resolved outside the repository.
use super::{error::*, types::validate_path};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeManifest {
    pub schema_version: super::types::SchemaVersion,
    pub node_version: String,
    pub node_sha256: String,
    pub files: BTreeMap<String, String>,
    pub browser: BrowserPin,
    pub fonts: Vec<FontPin>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FontPin {
    pub directory: PathBuf,
    pub files: BTreeMap<String, String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserPin {
    pub cache: PathBuf,
    pub directory: PathBuf,
    pub executable: PathBuf,
    pub files: BTreeMap<String, String>,
}
#[derive(Clone)]
pub struct TrustedDesignRuntime {
    pub(crate) node: PathBuf,
    pub(crate) root: PathBuf,
    manifest: RuntimeManifest,
    fingerprint: String,
}
pub(crate) fn file_hash(path: &Path) -> DesignResult<String> {
    use std::io::Read;
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
pub(crate) fn no_links(path: &Path) -> DesignResult<()> {
    for part in path.ancestors() {
        let meta = fs::symlink_metadata(part)?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err(DesignError::Denied("reparse point in trusted path".into()));
            }
        }
        if meta.file_type().is_symlink() {
            return Err(DesignError::Denied("symlink in trusted path".into()));
        }
    }
    Ok(())
}
/// Node's CommonJS entry resolver rejects Windows verbatim drive paths.
/// Use only after canonical-path authorization; never use this for comparisons.
pub(crate) fn node_path(path: &Path) -> String {
    let value = path.to_string_lossy().into_owned();
    #[cfg(windows)]
    {
        if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{unc}");
        }
        if let Some(drive) = value.strip_prefix(r"\\?\") {
            return drive.into();
        }
    }
    value
}
impl TrustedDesignRuntime {
    pub fn configured(workspace: &Path) -> DesignResult<Self> {
        let root = std::env::var_os("DAVINCI_DESIGN_RUNTIME").ok_or_else(|| {
            DesignError::MissingCapability(
                "install the design runtime and set DAVINCI_DESIGN_RUNTIME".into(),
            )
        })?;
        let node = std::env::var_os("DAVINCI_DESIGN_NODE").ok_or_else(|| {
            DesignError::MissingCapability(
                "set DAVINCI_DESIGN_NODE to the pinned executable".into(),
            )
        })?;
        Self::load(Path::new(&node), Path::new(&root), workspace)
    }
    pub(crate) fn browser(&self) -> &BrowserPin {
        &self.manifest.browser
    }
    /// Host configuration only. Browser/model arguments cannot select these paths.
    pub fn load(node: &Path, root: &Path, workspace: &Path) -> DesignResult<Self> {
        no_links(node)?;
        no_links(root)?;
        let node = node.canonicalize()?;
        let root = root.canonicalize()?;
        let workspace = workspace.canonicalize()?;
        if node.starts_with(&workspace) || root.starts_with(&workspace) {
            return Err(DesignError::Denied(
                "design runtime must be installed outside the workspace".into(),
            ));
        }
        let path = root.join("runtime-manifest.json");
        no_links(&path)?;
        if fs::metadata(&path)?.len() > 2 * 1024 * 1024 {
            return Err(DesignError::BudgetExceeded("runtime manifest size".into()));
        }
        let bytes = fs::read(path)?;
        let manifest: RuntimeManifest = serde_json::from_slice(&bytes)?;
        let fingerprint = format!("{:x}", Sha256::digest(&bytes));
        let runtime = Self {
            node,
            root,
            manifest,
            fingerprint,
        };
        if runtime
            .manifest
            .browser
            .cache
            .canonicalize()?
            .starts_with(&workspace)
        {
            return Err(DesignError::Denied(
                "browser cache must be outside the workspace".into(),
            ));
        }
        for font in &runtime.manifest.fonts {
            no_links(&font.directory)?;
            if font.directory.canonicalize()?.starts_with(&workspace) {
                return Err(DesignError::Denied(
                    "pinned fonts must be outside the workspace".into(),
                ));
            }
        }
        runtime.verify()?;
        Ok(runtime)
    }
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
    pub fn verify(&self) -> DesignResult<()> {
        no_links(&self.node)?;
        no_links(&self.root)?;
        if file_hash(&self.root.join("runtime-manifest.json"))? != self.fingerprint {
            return Err(DesignError::CorruptArtifact(
                "runtime manifest changed".into(),
            ));
        }
        if self.manifest.node_version != "24.19.0"
            || file_hash(&self.node)? != self.manifest.node_sha256
        {
            return Err(DesignError::MissingCapability(
                "pinned Node runtime does not match installed manifest".into(),
            ));
        }
        if self.manifest.files.len() > 5000 {
            return Err(DesignError::BudgetExceeded("runtime file count".into()));
        }
        for required in [
            "host/main.cjs",
            "host/server.cjs",
            "host/protocol.cjs",
            "host/compiler.cjs",
            "host/compile.cjs",
            "ui/index.html",
            "ui/app.js",
            "ui/app.css",
            "ui/manifest.json",
        ] {
            if !self.manifest.files.contains_key(required) {
                return Err(DesignError::MissingCapability(format!(
                    "runtime is missing {required}"
                )));
            }
        }
        for (name, hash) in &self.manifest.files {
            validate_path(name)?;
            let path = self.root.join(name);
            no_links(&path)?;
            if !fs::metadata(&path)?.is_file() || file_hash(&path)? != *hash {
                return Err(DesignError::CorruptArtifact(
                    "installed design runtime changed; reinstall explicitly".into(),
                ));
            }
        }
        let mut pending = vec![self.root.clone()];
        let mut count = 0usize;
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(directory)? {
                let path = entry?.path();
                no_links(&path)?;
                let metadata = fs::symlink_metadata(&path)?;
                if metadata.is_dir() {
                    pending.push(path);
                } else if metadata.is_file() {
                    let relative = path
                        .strip_prefix(&self.root)
                        .map_err(|_| DesignError::Denied("runtime path escaped".into()))?
                        .to_string_lossy()
                        .replace('\\', "/");
                    if relative == "runtime-manifest.json" {
                        continue;
                    }
                    if !self.manifest.files.contains_key(&relative) {
                        return Err(DesignError::CorruptArtifact(
                            "unlisted file in design runtime".into(),
                        ));
                    }
                    count += 1;
                } else {
                    return Err(DesignError::Denied("nonregular runtime entry".into()));
                }
                if pending.len() > 5000 {
                    return Err(DesignError::BudgetExceeded(
                        "runtime directory count".into(),
                    ));
                }
            }
        }
        if count != self.manifest.files.len() {
            return Err(DesignError::CorruptArtifact(
                "runtime inventory mismatch".into(),
            ));
        }
        let browser = &self.manifest.browser;
        for path in [&browser.cache, &browser.directory, &browser.executable] {
            if !path.is_absolute() {
                return Err(DesignError::Denied(
                    "browser pin must use absolute paths".into(),
                ));
            }
            no_links(path)?;
        }
        if !browser.directory.starts_with(&browser.cache)
            || !browser.executable.starts_with(&browser.directory)
            || browser.files.is_empty()
            || browser.files.len() > 5000
        {
            return Err(DesignError::Denied(
                "browser pin is outside its cache".into(),
            ));
        }
        let executable = browser
            .executable
            .strip_prefix(&browser.directory)
            .map_err(|_| DesignError::Denied("browser pin path".into()))?
            .to_string_lossy()
            .replace('\\', "/");
        if !browser.files.contains_key(&executable) {
            return Err(DesignError::CorruptArtifact(
                "browser executable is not pinned".into(),
            ));
        }
        for (name, hash) in &browser.files {
            // Installed browser resources include spaces (for example "First Run").
            // They are trusted inventory names, not design source filenames.
            if name.is_empty()
                || name.contains(['\\', ':', '\0'])
                || name.split('/').any(|part| {
                    part.is_empty()
                        || part == "."
                        || part == ".."
                        || part.chars().any(char::is_control)
                })
            {
                return Err(DesignError::Denied("invalid browser inventory path".into()));
            }
            let path = browser.directory.join(name);
            no_links(&path)?;
            if file_hash(&path)? != *hash {
                return Err(DesignError::CorruptArtifact(
                    "browser installation changed".into(),
                ));
            }
        }
        verify_inventory(&browser.directory, &browser.files)?;
        if self.manifest.fonts.is_empty() || self.manifest.fonts.len() > 8 {
            return Err(DesignError::MissingCapability(
                "pin the installed system font inventory".into(),
            ));
        }
        for fonts in &self.manifest.fonts {
            verify_inventory(&fonts.directory, &fonts.files)?;
        }
        Ok(())
    }
}

fn verify_inventory(root: &Path, files: &BTreeMap<String, String>) -> DesignResult<()> {
    if !root.is_absolute() || files.len() > 5000 {
        return Err(DesignError::Denied(
            "invalid installed inventory root or file count".into(),
        ));
    }
    no_links(root)?;
    let mut pending = vec![root.to_path_buf()];
    let mut seen = 0usize;
    let mut directories = 0usize;
    while let Some(directory) = pending.pop() {
        directories += 1;
        if directories > 5000 {
            return Err(DesignError::BudgetExceeded(
                "installed inventory directory count".into(),
            ));
        }
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            no_links(&path)?;
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.is_dir() {
                pending.push(path);
                continue;
            }
            if !metadata.is_file() {
                return Err(DesignError::Denied(
                    "nonregular installed inventory file".into(),
                ));
            }
            let name = path
                .strip_prefix(root)
                .map_err(|_| DesignError::Denied("inventory path escaped".into()))?
                .to_string_lossy()
                .replace('\\', "/");
            let hash = files.get(&name).ok_or_else(|| {
                DesignError::CorruptArtifact("unlisted browser or font file".into())
            })?;
            if file_hash(&path)? != *hash {
                return Err(DesignError::CorruptArtifact(
                    "browser or font inventory changed".into(),
                ));
            }
            seen += 1;
        }
    }
    if seen != files.len() {
        return Err(DesignError::CorruptArtifact(
            "installed inventory file missing".into(),
        ));
    }
    Ok(())
}
