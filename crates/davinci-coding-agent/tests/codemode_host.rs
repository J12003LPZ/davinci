use davinci_coding_agent::codemode_host::assets::{
    validate_node_version, AssetEntry, AssetManifest, HostAssets,
};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

fn fixture(root: &Path) -> AssetManifest {
    let assets = ["host.mjs", "bounded-worker.mjs", "quickjs.wasm"]
        .into_iter()
        .map(|path| {
            fs::write(root.join(path), path.as_bytes()).unwrap();
            AssetEntry {
                path: path.into(),
                bytes: path.len() as u64,
                sha256: format!(
                    "{:x}",
                    davinci_sys::hex::Lower(&Sha256::digest(path.as_bytes()))
                ),
            }
        })
        .collect();
    AssetManifest {
        schema_version: 1,
        protocol_version: 1,
        node_version: "24.21.0".into(),
        pi_version: "1.0.2".into(),
        quickjs_version: "3.6.2".into(),
        entry: "host.mjs".into(),
        worker: "bounded-worker.mjs".into(),
        wasm: "quickjs.wasm".into(),
        assets,
    }
}

#[test]
fn valid_assets_have_canonical_host_identity() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = fixture(dir.path());
    let verified = HostAssets::validate(dir.path(), &manifest).unwrap();
    assert_eq!(
        verified.entry(),
        dir.path().canonicalize().unwrap().join("host.mjs")
    );
}

#[test]
fn rejects_asset_hash_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = fixture(dir.path());
    fs::write(dir.path().join("host.mjs"), b"evil.mjs").unwrap();
    assert!(HostAssets::validate(dir.path(), &manifest).is_err());
}

#[test]
fn rejects_project_host_override() {
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().join("host");
    fs::create_dir(&root).unwrap();
    let manifest = fixture(&root);
    assert!(HostAssets::validate_installation(&root, workspace.path(), &manifest).is_err());
}

#[test]
fn rejects_asset_escape() {
    let dir = tempfile::tempdir().unwrap();
    let mut manifest = fixture(dir.path());
    manifest.assets[0].path = "../host.mjs".into();
    assert!(HostAssets::validate(dir.path(), &manifest).is_err());
    manifest.assets[0].path = "C:/host.mjs".into();
    assert!(HostAssets::validate(dir.path(), &manifest).is_err());
}

#[test]
fn rejects_missing_worker_and_unlisted_entry() {
    let dir = tempfile::tempdir().unwrap();
    let mut manifest = fixture(dir.path());
    fs::remove_file(dir.path().join("bounded-worker.mjs")).unwrap();
    assert!(HostAssets::validate(dir.path(), &manifest).is_err());
    manifest.entry = "unlisted.mjs".into();
    assert!(HostAssets::validate(dir.path(), &manifest).is_err());
}

#[test]
fn rejects_extra_executable_asset() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = fixture(dir.path());
    fs::write(dir.path().join("preload.js"), b"extra").unwrap();
    assert!(HostAssets::validate(dir.path(), &manifest).is_err());
}

#[test]
fn rejects_incompatible_node() {
    assert!(validate_node_version("v24.21.0\n").is_ok());
    for version in [
        "v24.19.0",
        "v22.19.0",
        "v25.0.0",
        "v24.21.1",
        "v24.21.0-extra",
    ] {
        assert!(validate_node_version(version).is_err(), "{version}");
    }
}

#[test]
fn rejects_protocol_or_package_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let mut manifest = fixture(dir.path());
    manifest.protocol_version = 2;
    assert!(HostAssets::validate(dir.path(), &manifest).is_err());
    manifest.protocol_version = 1;
    manifest.pi_version = "latest".into();
    assert!(HostAssets::validate(dir.path(), &manifest).is_err());
}

#[test]
fn spaces_and_unicode_are_supported() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("host space 界");
    fs::create_dir(&root).unwrap();
    let manifest = fixture(&root);
    assert!(HostAssets::validate(&root, &manifest).is_ok());
}

#[cfg(windows)]
#[test]
fn rejects_junction_escape() {
    use std::process::Command;
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let mut manifest = fixture(dir.path());
    let link = dir.path().join("junction");
    // Only this disposable fixture uses cmd's junction builtin; host launch never does.
    let status = Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&link)
        .arg(outside.path())
        .status()
        .unwrap();
    assert!(status.success());
    manifest.assets[0].path = "junction/host.mjs".into();
    assert!(HostAssets::validate(dir.path(), &manifest).is_err());
}

#[cfg(unix)]
#[test]
fn rejects_symlink_escape() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let manifest = fixture(dir.path());
    fs::remove_file(dir.path().join("host.mjs")).unwrap();
    fs::write(outside.path().join("host.mjs"), b"host.mjs").unwrap();
    std::os::unix::fs::symlink(outside.path().join("host.mjs"), dir.path().join("host.mjs"))
        .unwrap();
    assert!(HostAssets::validate(dir.path(), &manifest).is_err());
}
