pub fn allow_tool(name: &str, approved: bool) -> bool {
    approved && matches!(name, "write" | "bash" | "edit")
}
