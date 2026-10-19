//! "Trust parent folder" must never offer a grant that covers the user's whole
//! home directory, a drive root, or anything above them. This test sets
//! HOME for its own process, so it lives in a file of its own.

use davinci_coding_agent::trust::{
    get_project_trust_options, get_project_trust_parent_path, has_trust_requiring_project_resources,
};
use std::fs;
use std::sync::Mutex;

/// Both tests set HOME for the process, so they take turns.
static HOME: Mutex<()> = Mutex::new(());

#[test]
fn trust_parent_is_not_offered_at_home_a_root_or_above() {
    let _turn = HOME.lock().unwrap_or_else(|e| e.into_inner());
    let root = tempfile::tempdir().unwrap();
    let home = fs::canonicalize(root.path()).unwrap().join("home");
    let work = home.join("work");
    let project = work.join("repo");
    fs::create_dir_all(&project).unwrap();
    std::env::set_var("HOME", &home);
    std::env::set_var("USERPROFILE", &home);

    // Directly under home: the parent is the home directory itself.
    let direct = home.join("repo");
    fs::create_dir_all(&direct).unwrap();
    assert_eq!(get_project_trust_parent_path(&direct), None);
    // One level deeper the parent is a folder inside home, which is fine.
    let parent = get_project_trust_parent_path(&project).expect("a work folder is a fair parent");
    assert!(parent.ends_with("work"), "{parent}");
    // A parent that is an ancestor of home is as broad as home.
    assert_eq!(get_project_trust_parent_path(&home), None);
    // A filesystem or drive root is never offered, and no option is built.
    let anchor = fs::canonicalize(root.path())
        .unwrap()
        .ancestors()
        .last()
        .unwrap()
        .to_path_buf();
    let top_level = anchor.join("some-top-level-dir");
    assert_eq!(get_project_trust_parent_path(&top_level), None);
    assert!(!get_project_trust_options(&direct, false)
        .iter()
        .any(|option| option.label.starts_with("Trust parent folder")));
}

#[test]
fn the_project_search_stops_at_the_home_directory() {
    let _turn = HOME.lock().unwrap_or_else(|e| e.into_inner());
    let root = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(root.path()).unwrap();
    let home = base.join("home");
    let project = home.join("project");
    fs::create_dir_all(&project).unwrap();
    // `.agents/skills` above the home directory belongs to the user.
    fs::create_dir_all(base.join(".agents").join("skills")).unwrap();
    fs::create_dir_all(home.join(".agents").join("skills")).unwrap();
    std::env::set_var("HOME", &home);
    std::env::set_var("USERPROFILE", &home);
    assert!(!has_trust_requiring_project_resources(&project));
    fs::create_dir_all(project.join(".agents").join("skills")).unwrap();
    assert!(has_trust_requiring_project_resources(&project));
}
