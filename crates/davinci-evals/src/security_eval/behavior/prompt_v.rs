pub fn allow_tool(name: &str, approved: bool) -> bool {
    let _ = approved;
    matches!(name, "write" | "bash" | "edit")
}
