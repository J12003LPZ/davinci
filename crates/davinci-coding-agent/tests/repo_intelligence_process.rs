use davinci_coding_agent::native_extensions::repo_intelligence::RepoIntelligence;
use std::{fs, process::Command};

#[test]
fn repo_intelligence_process_worker() {
    let Ok(root) = std::env::var("DAVINCI_REPO_TEST_ROOT") else {
        return;
    };
    let cache = std::env::var("DAVINCI_REPO_TEST_CACHE").unwrap();
    let output = std::env::var("DAVINCI_REPO_TEST_OUTPUT").unwrap();
    let manager = RepoIntelligence::new(root.as_ref(), cache.as_ref(), Default::default());
    let index = manager.refresh().unwrap();
    assert_eq!(index.files.len(), 40);
    fs::write(output, index.reparsed.to_string()).unwrap();
}

#[test]
fn repo_intelligence_processes_share_persistent_index() {
    let repo = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    for i in 0..40 {
        fs::write(
            repo.path().join(format!("source{i}.ts")),
            format!("export function source{i}() {{ return {i}; }}"),
        )
        .unwrap();
    }
    let mut children = Vec::new();
    for i in 0..4 {
        children.push(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "repo_intelligence_process_worker", "--nocapture"])
                .env("DAVINCI_REPO_TEST_ROOT", repo.path())
                .env("DAVINCI_REPO_TEST_CACHE", cache.path())
                .env(
                    "DAVINCI_REPO_TEST_OUTPUT",
                    cache.path().join(format!("worker{i}.txt")),
                )
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
    }
    for child in &mut children {
        assert!(child.wait().unwrap().success());
    }
    let reparsed: usize = (0..4)
        .map(|i| {
            fs::read_to_string(cache.path().join(format!("worker{i}.txt")))
                .unwrap()
                .parse::<usize>()
                .unwrap()
        })
        .sum();
    assert_eq!(
        reparsed, 40,
        "one cold index across independent worker processes"
    );
}

#[test]
fn repo_intelligence_readers_during_refresh_keep_consistent_records() {
    let repo = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let path = repo.path().join("entry.ts");
    fs::write(&path, "export function before() {}").unwrap();
    let manager = RepoIntelligence::new(repo.path(), cache.path(), Default::default());
    let before = manager.refresh().unwrap();
    fs::write(&path, "export function after() {}").unwrap();
    std::thread::scope(|scope| {
        let jobs: Vec<_> = (0..4)
            .map(|_| scope.spawn(|| manager.refresh().unwrap()))
            .collect();
        for job in jobs {
            let index = job.join().unwrap();
            assert_eq!(index.files["entry.ts"].symbols[0].name, "after");
        }
    });
    assert_eq!(
        before.files["entry.ts"].symbols[0].name, "before",
        "published snapshots are immutable"
    );
}
