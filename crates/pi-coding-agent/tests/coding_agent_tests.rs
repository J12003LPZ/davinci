use pi_agent::agent::AgentTool;
use pi_coding_agent::commands::builtin_slash_commands;
use pi_coding_agent::tools::{BashTool, EditTool, ReadTool, WriteTool};
use serde_json::json;
use tempfile::tempdir;

#[tokio::test]
async fn test_coding_agent_tools() {
    let tmp = tempdir().expect("tempdir");
    let file_path = tmp.path().join("test.txt");
    let file_path_str = file_path.to_string_lossy().to_string();

    let write_tool = WriteTool;
    write_tool
        .execute(
            "w1",
            json!({
                "path": file_path_str,
                "content": "Hello Rust Pi"
            }),
        )
        .await
        .expect("write tool");

    let read_tool = ReadTool;
    let read_res = read_tool
        .execute("r1", json!({ "path": file_path_str }))
        .await
        .expect("read tool");
    assert!(!read_res.content.is_empty());

    let edit_tool = EditTool;
    edit_tool
        .execute(
            "e1",
            json!({
                "path": file_path_str,
                "edits": [{
                    "oldText": "Rust Pi",
                    "newText": "World"
                }]
            }),
        )
        .await
        .expect("edit tool");

    let read_res2 = read_tool
        .execute("r2", json!({ "path": file_path_str }))
        .await
        .expect("read tool 2");
    match &read_res2.content[0] {
        pi_ai::types::UserContent::Text(t) => assert_eq!(t.text, "Hello World"),
        _ => panic!("Expected text"),
    }

    let bash_tool = BashTool;
    let bash_res = bash_tool
        .execute("b1", json!({ "command": "echo 'pi test'" }))
        .await
        .expect("bash tool");
    match &bash_res.content[0] {
        pi_ai::types::UserContent::Text(t) => assert!(t.text.contains("pi test")),
        _ => panic!("Expected text"),
    }
}

#[test]
fn test_builtin_slash_commands() {
    let commands = builtin_slash_commands();
    assert!(commands.iter().any(|c| c.name == "settings"));
    assert!(commands.iter().any(|c| c.name == "model"));
    assert!(commands.iter().any(|c| c.name == "thinking"));
    assert!(commands.iter().any(|c| c.name == "export"));
    assert!(commands.iter().any(|c| c.name == "compact"));
    assert!(commands.iter().any(|c| c.name == "resume"));
}
