//! Regressions for the Git Intelligence audit findings (WOR-216, 218-222).
use davinci_coding_agent::native_extensions::git_intelligence::{
    model::GitIntelligenceConfig, resolver::GitResolver,
};
use std::{path::Path, process::Command};
use tempfile::TempDir;

struct TestRepo {
    dir: TempDir,
}

impl TestRepo {
    fn new() -> Self {
        let repo = Self {
            dir: TempDir::new().unwrap(),
        };
        repo.git(&["init", "-b", "main"]);
        repo.git(&["config", "user.name", "Test Committer"]);
        repo.git(&["config", "user.email", "committer@example.com"]);
        repo.git(&["config", "commit.gpgsign", "false"]);
        repo
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .current_dir(self.path())
            .args(args)
            .output()
            .expect("git command failed");
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    fn write_file(&self, rel_path: &str, content: &str) {
        let full = self.path().join(rel_path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full, content).unwrap();
    }

    fn commit(&self, message: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-m", message]);
        self.git(&["rev-parse", "HEAD"]).trim().to_string()
    }

    fn commit_as(&self, author: &str, message: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&[
            "-c",
            &format!("user.name={author}"),
            "commit",
            "-m",
            message,
        ]);
        self.git(&["rev-parse", "HEAD"]).trim().to_string()
    }

    fn resolver(&self) -> GitResolver {
        GitResolver::new(self.path(), GitIntelligenceConfig::default()).unwrap()
    }
}

#[test]
fn body_only_edit_on_the_same_lines_is_a_modified_symbol() {
    let repo = TestRepo::new();
    repo.write_file("src/a.js", "function answer(){return 1}\n");
    let base = repo.commit("base");
    repo.write_file("src/a.js", "function answer(){return 2}\n");
    let head = repo.commit("body");
    let result = repo
        .resolver()
        .changed_symbols(Some(&base), Some(&head), None)
        .unwrap();
    assert!(
        result
            .symbols
            .iter()
            .any(|symbol| symbol.name == "answer" && symbol.change_type == "modified"),
        "{result:?}"
    );
}

#[test]
fn unchanged_symbols_are_not_reported_modified() {
    let repo = TestRepo::new();
    repo.write_file(
        "src/a.js",
        "function keep(){return 1}\nfunction edit(){return 1}\n",
    );
    let base = repo.commit("base");
    repo.write_file(
        "src/a.js",
        "function keep(){return 1}\nfunction edit(){return 2}\n",
    );
    let head = repo.commit("edit");
    let result = repo
        .resolver()
        .changed_symbols(Some(&base), Some(&head), None)
        .unwrap();
    let names: Vec<_> = result.symbols.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["edit"]);
}

#[test]
fn paths_with_spaces_survive_changed_symbols_and_branch_diff() {
    let repo = TestRepo::new();
    repo.write_file("src/with space.js", "function spaced(){return 1}\n");
    let base = repo.commit("base");
    repo.write_file("src/with space.js", "function spaced(){return 2}\n");
    let head = repo.commit("edit");

    let changed = repo
        .resolver()
        .changed_symbols(Some(&base), Some(&head), None)
        .unwrap();
    assert!(
        changed
            .symbols
            .iter()
            .any(|s| s.name == "spaced" && s.file == "src/with space.js"),
        "{changed:?}"
    );

    let diff = repo
        .resolver()
        .branch_diff(Some(&base), Some(&head), Some(false), None)
        .unwrap();
    assert_eq!(diff.files.len(), 1);
    assert_eq!(diff.files[0].path, "src/with space.js");
    assert!(diff.files[0]
        .patch_snippet
        .as_deref()
        .is_some_and(|p| p.contains("return 2")));
}

#[test]
fn branch_diff_reports_the_true_file_count_beyond_max_files() {
    let repo = TestRepo::new();
    repo.write_file("README.md", "base\n");
    let base = repo.commit("base");
    for index in 0..8 {
        repo.write_file(&format!("f{index}.txt"), "x\n");
    }
    let head = repo.commit("many");
    let diff = repo
        .resolver()
        .branch_diff(Some(&base), Some(&head), Some(true), Some(3))
        .unwrap();
    assert_eq!(diff.files.len(), 3);
    assert_eq!(diff.files_changed, 8);
    assert_eq!(diff.insertions, 8);

    let none = repo
        .resolver()
        .branch_diff(Some(&base), Some(&head), Some(true), Some(0))
        .unwrap();
    assert_eq!(none.files_changed, 8);
}

#[test]
fn blame_keeps_authors_for_repeated_commit_hunks() {
    let repo = TestRepo::new();
    repo.write_file(
        "src/a.js",
        "function f(){\n  const a = 1;\n  const b = 2;\n  const c = 3;\n  return a+b+c;\n}\n",
    );
    repo.commit_as("Alice", "alice");
    repo.write_file(
        "src/a.js",
        "function f(){\n  const a = 1;\n  const b = 20;\n  const c = 3;\n  return a+b+c;\n}\n",
    );
    repo.commit_as("Bob", "bob");
    let blame = repo.resolver().blame_symbol("f", "src/a.js", None).unwrap();
    let authors: Vec<_> = blame.lines.iter().map(|l| l.author.as_str()).collect();
    assert_eq!(
        authors,
        ["Alice", "Alice", "Bob", "Alice", "Alice", "Alice"]
    );
}

#[test]
fn symbol_history_keeps_the_rename_commit_under_its_new_path() {
    let repo = TestRepo::new();
    repo.write_file("src/foo.js", "function keep(){return 1}\n");
    repo.commit("add");
    repo.git(&["mv", "src/foo.js", "src/bar.js"]);
    let rename = repo.commit("rename");
    let history = repo
        .resolver()
        .symbol_history("keep", Some("src/bar.js"), None)
        .unwrap();
    let commits: Vec<_> = history
        .modifications
        .iter()
        .chain(history.introduced_commit.iter())
        .map(|fact| fact.commit.as_str())
        .collect();
    assert!(commits.contains(&rename.as_str()), "{history:?}");
}

#[test]
fn merge_commit_context_reports_changes_against_the_first_parent() {
    let repo = TestRepo::new();
    repo.write_file("base.txt", "base\n");
    repo.commit("base");
    repo.git(&["checkout", "-b", "feature"]);
    repo.write_file("feature.js", "function feature(){return 1}\n");
    repo.commit("feature");
    repo.git(&["checkout", "main"]);
    repo.write_file("main.txt", "main\n");
    repo.commit("main");
    repo.git(&["merge", "--no-ff", "-m", "merge feature", "feature"]);
    let merge = repo.git(&["rev-parse", "HEAD"]).trim().to_string();

    let context = repo.resolver().commit_context(&merge).unwrap();
    assert_eq!(context.parents.len(), 2);
    assert_eq!(context.facts.files_changed, ["feature.js"]);
    assert_eq!(context.facts.insertions, 1);
}
