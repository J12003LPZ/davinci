use davinci_session::{build_session_tree, SessionEntry};
use serde_json::Value;

fn entry(id: usize, parent: Option<usize>) -> SessionEntry {
    let mut entry = SessionEntry::message("user", serde_json::json!("hi"));
    entry.id = format!("e{id}");
    entry.parent_id = parent.map(|parent| format!("e{parent}"));
    entry.seq = id as u64 + 1;
    entry.timestamp = id as u64;
    entry
}

fn count(nodes: &Value) -> usize {
    // Iterative: the test must not depend on the depth being bounded.
    let mut total = 0;
    let mut stack = vec![nodes];
    while let Some(value) = stack.pop() {
        for node in value.as_array().unwrap() {
            total += 1;
            stack.push(&node["children"]);
        }
    }
    total
}

#[test]
fn deep_linear_history_builds_without_overflowing_the_stack() {
    let entries: Vec<_> = (0..10_000).map(|id| entry(id, id.checked_sub(1))).collect();
    let (total, flattened) = std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(move || {
            let tree = build_session_tree(&entries);
            let json = serde_json::to_string(&tree).unwrap();
            (count(&tree), json.contains("\"flattened\":true"))
        })
        .unwrap()
        .join()
        .expect("deep tree must not overflow the thread stack");
    assert_eq!(total, 10_000, "every entry stays in the tree");
    assert!(
        flattened,
        "entries below the depth cap are marked flattened"
    );
}

#[test]
fn shallow_tree_keeps_nesting_and_sibling_order() {
    let mut entries = vec![entry(0, None), entry(1, Some(0)), entry(2, Some(0))];
    entries[1].timestamp = 20;
    entries[2].timestamp = 10;
    entries.push(entry(3, Some(1)));
    let tree = build_session_tree(&entries);
    let root = &tree[0];
    assert_eq!(root["entry"]["id"], "e0");
    assert_eq!(root["children"][0]["entry"]["id"], "e2");
    assert_eq!(root["children"][1]["entry"]["id"], "e1");
    assert_eq!(root["children"][1]["children"][0]["entry"]["id"], "e3");
    assert!(root["children"][1].get("flattened").is_none());
}
