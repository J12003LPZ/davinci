//! Validate installed assets against a manifest supplied by the trusted Rust build.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetEntry {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetManifest {
    pub schema_version: u32,
    pub protocol_version: u32,
    pub node_version: String,
    pub pi_version: String,
    pub quickjs_version: String,
    pub entry: String,
    pub worker: String,
    pub wasm: String,
    pub assets: Vec<AssetEntry>,
}

/// Embedded in the trusted executable; an adjacent file cannot override it.
pub fn trusted_manifest() -> std::io::Result<AssetManifest> {
    serde_json::from_str(include_str!("assets-manifest.json"))
        .map_err(|_| invalid("invalid built-in host manifest"))
}

#[derive(Debug)]
pub struct HostAssets {
    root: PathBuf,
    entry: PathBuf,
    worker: PathBuf,
    wasm: PathBuf,
}

fn invalid(message: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message)
}

pub fn validate_node_version(version: &str) -> std::io::Result<()> {
    if version.trim_end_matches(['\r', '\n']) != "v24.21.0" {
        return Err(invalid(
            "Codemode requires the admitted Node 24.21.0 runtime",
        ));
    }
    Ok(())
}

fn regular_path(root: &Path, relative: &str) -> std::io::Result<PathBuf> {
    if relative.is_empty() || relative.contains(['\\', ':']) {
        return Err(invalid("invalid asset path"));
    }
    let mut path = root.to_path_buf();
    for component in Path::new(relative).components() {
        let Component::Normal(part) = component else {
            return Err(invalid(
                "asset path must be relative and contain no traversal",
            ));
        };
        path.push(part);
        reject_link(&path)?;
    }
    Ok(path)
}

pub(super) fn reject_link(path: &Path) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(invalid("linked host assets are not admitted"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(invalid("reparse-point host assets are not admitted"));
        }
    }
    Ok(())
}

fn inventory(root: &Path, directory: &Path, paths: &mut BTreeSet<String>) -> std::io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        reject_link(&path)?;
        if entry.file_type()?.is_dir() {
            inventory(root, &path, paths)?;
        } else if entry.file_type()?.is_file() {
            paths.insert(
                path.strip_prefix(root)
                    .map_err(|_| invalid("asset escape"))?
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        } else {
            return Err(invalid("unsupported host asset type"));
        }
    }
    Ok(())
}

impl HostAssets {
    /// `manifest` must originate from the trusted build, never from the installation.
    pub fn validate(root: &Path, manifest: &AssetManifest) -> std::io::Result<Self> {
        if manifest.schema_version != 1
            || manifest.protocol_version != 1
            || manifest.node_version != "24.21.0"
            || manifest.pi_version != "1.0.2"
            || manifest.quickjs_version != "3.6.2"
            || manifest.assets.is_empty()
            || manifest.assets.len() > 128
        {
            return Err(invalid("unsupported Codemode asset manifest"));
        }
        if !root.is_absolute() {
            return Err(invalid("host installation must be absolute"));
        }
        for ancestor in root.ancestors() {
            reject_link(ancestor)?;
        }
        let root = root.canonicalize()?;
        let mut expected = BTreeSet::new();
        for asset in &manifest.assets {
            let path = regular_path(&root, &asset.path)?;
            if !expected.insert(asset.path.clone())
                || asset.bytes > 64 * 1024 * 1024
                || asset.sha256.len() != 64
                || !path.is_file()
                || fs::metadata(&path)?.len() != asset.bytes
            {
                return Err(invalid("invalid or missing Codemode asset"));
            }
            let mut file = fs::File::open(path)?;
            let mut hash = Sha256::new();
            let mut buffer = [0u8; 16384];
            let mut bytes = 0u64;
            loop {
                let count = file.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                bytes += count as u64;
                if bytes > asset.bytes {
                    return Err(invalid("asset changed during admission"));
                }
                hash.update(&buffer[..count]);
            }
            if bytes != asset.bytes
                || format!("{:x}", davinci_sys::hex::Lower(&hash.finalize())) != asset.sha256
            {
                return Err(invalid("Codemode asset digest mismatch"));
            }
        }
        let mut actual = BTreeSet::new();
        inventory(&root, &root, &mut actual)?;
        if actual != expected {
            return Err(invalid("unlisted Codemode assets"));
        }
        for required in [&manifest.entry, &manifest.worker, &manifest.wasm] {
            if !expected.contains(required) {
                return Err(invalid("unlisted host entry point"));
            }
        }
        Ok(Self {
            entry: root.join(&manifest.entry),
            worker: root.join(&manifest.worker),
            wasm: root.join(&manifest.wasm),
            root,
        })
    }

    /// `trusted_root` is DaVinci's own runtime directory (`<agent dir>/codemode`).
    /// A host under it is admitted even when the workspace contains it, as when
    /// DaVinci runs from the home directory; any other host in the workspace is
    /// project-provided and refused.
    pub fn validate_installation(
        root: &Path,
        workspace: &Path,
        trusted_root: Option<&Path>,
        manifest: &AssetManifest,
    ) -> std::io::Result<Self> {
        let assets = Self::validate(root, manifest)?;
        let trusted = trusted_root.and_then(|path| path.canonicalize().ok());
        if assets.root.starts_with(workspace.canonicalize()?)
            && !trusted.is_some_and(|trusted| assets.root.starts_with(trusted))
        {
            return Err(invalid("workspace-provided hosts are not admitted"));
        }
        Ok(assets)
    }

    pub fn entry(&self) -> &Path {
        &self.entry
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn worker(&self) -> &Path {
        &self.worker
    }
    pub fn wasm(&self) -> &Path {
        &self.wasm
    }
}
