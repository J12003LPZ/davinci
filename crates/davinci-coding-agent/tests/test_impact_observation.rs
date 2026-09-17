use davinci_coding_agent::native_extensions::repo_intelligence::{
    RepoIntelligence, RepoIntelligenceConfig,
};
use std::{fs, process::Command};

#[test]
fn observation_external_writer() {
    let Ok(path) = std::env::var("DAVINCI_IMPACT_WRITE_FIXTURE") else {
        return;
    };
    let time = fs::metadata(&path).unwrap().modified().unwrap();
    fs::write(&path, "export const after = 2;").unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(time))
        .unwrap();
}

#[test]
fn observed_external_same_size_timestamp_preserving_write_is_invalidated() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let path = root.path().join("source.ts");
    fs::write(&path, "export const prior = 1;").unwrap();
    fs::write(root.path().join("input.ts"), "import './source';").unwrap();
    let repo = RepoIntelligence::new(root.path(), cache.path(), Default::default());
    let before = repo.refresh().unwrap();
    assert!(Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "observation_external_writer"])
        .env("DAVINCI_IMPACT_WRITE_FIXTURE", &path)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap()
        .success());
    let after = repo
        .refresh_observed_authorized(&["input.ts".into()], false, &|_| Ok(()))
        .unwrap();
    assert_ne!(
        after.files["source.ts"].content_hash,
        before.files["source.ts"].content_hash
    );
    assert_eq!(after.files["source.ts"].symbols[0].name, "after");
    assert_eq!(before.files["source.ts"].symbols[0].name, "prior");
}

#[test]
fn observed_inventory_reconciles_add_delete_rename_and_ignore_changes() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    fs::write(root.path().join("before.ts"), "export const before = 1;").unwrap();
    let repo = RepoIntelligence::new(root.path(), cache.path(), Default::default());
    repo.refresh().unwrap();
    fs::create_dir(root.path().join("nested")).unwrap();
    fs::rename(
        root.path().join("before.ts"),
        root.path().join("nested/after.ts"),
    )
    .unwrap();
    fs::write(root.path().join("created.ts"), "export const created = 1;").unwrap();
    let first = repo
        .refresh_observed_authorized(&[], false, &|_| Ok(()))
        .unwrap();
    assert_eq!(
        first.files.keys().map(String::as_str).collect::<Vec<_>>(),
        ["created.ts", "nested/after.ts"]
    );
    fs::remove_file(root.path().join("created.ts")).unwrap();
    fs::write(root.path().join(".gitignore"), "nested/\n").unwrap();
    let next = repo
        .refresh_observed_authorized(&[], false, &|_| Ok(()))
        .unwrap();
    assert!(next.files.is_empty());
    assert_eq!(next.refresh_mode, "full");
}

#[test]
fn observation_disabled_falls_back_and_workspaces_do_not_share_snapshots() {
    let one = tempfile::tempdir().unwrap();
    let two = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    fs::write(one.path().join("source.ts"), "export const one = 1;").unwrap();
    fs::write(two.path().join("source.ts"), "export const two = 2;").unwrap();
    let config = RepoIntelligenceConfig {
        observe_changes: false,
        ..Default::default()
    };
    let first = RepoIntelligence::new(one.path(), cache.path(), config.clone());
    let second = RepoIntelligence::new(two.path(), cache.path(), config);
    first.refresh().unwrap();
    let warm = first
        .refresh_observed_authorized(&[], false, &|_| Ok(()))
        .unwrap();
    assert_eq!(warm.refresh_mode, "full");
    assert_eq!(warm.files_read, 1);
    assert_eq!(warm.files["source.ts"].symbols[0].name, "one");
    assert_eq!(
        second.refresh().unwrap().files["source.ts"].symbols[0].name,
        "two"
    );
    assert_eq!(first.status()["observation"]["active"], false);
}

#[test]
fn interrupted_authorization_does_not_publish_or_poison_shared_refresh() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    fs::write(root.path().join("source.ts"), "export const value = 1;").unwrap();
    let repo = RepoIntelligence::new(root.path(), cache.path(), Default::default());
    assert!(repo
        .refresh_observed_authorized(&[], false, &|_| Err("cancelled".into()))
        .is_err());
    assert_eq!(repo.status()["initialized"], false);
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..4)
            .map(|_| {
                scope.spawn(|| {
                    repo.refresh_observed_authorized(&[], false, &|_| Ok(()))
                        .unwrap()
                })
            })
            .collect();
        let results: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(results.iter().map(|r| r.reparsed).sum::<usize>(), 1);
        assert!(results.iter().all(|r| r.files.len() == 1));
    });
}

#[test]
fn interrupted_inventory_requires_full_reconciliation_on_next_request() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    fs::write(root.path().join(".gitignore"), "ignored/\n").unwrap();
    fs::write(root.path().join("source.ts"), "export const value = 1;").unwrap();
    let repo = RepoIntelligence::new(root.path(), cache.path(), Default::default());
    repo.refresh().unwrap();
    assert!(repo
        .refresh_observed_authorized(&[], false, &|path| {
            if path == ".gitignore" {
                Err("cancelled".into())
            } else {
                Ok(())
            }
        })
        .is_err());
    let recovered = repo
        .refresh_observed_authorized(&[], false, &|_| Ok(()))
        .unwrap();
    assert_eq!(recovered.refresh_mode, "full");
    assert_eq!(recovered.files_read, 1);
}
