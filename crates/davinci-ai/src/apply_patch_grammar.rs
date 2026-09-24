//! Lark grammar for the freeform `apply_patch` tool.
//! Mirrors the patch forms accepted by DaVinci's Codex patch parser.

pub const APPLY_PATCH_LARK: &str = r#"start: begin_patch hunk+ end_patch
begin_patch: "*** Begin Patch" LF
end_patch: "*** End Patch" LF?

hunk: add_hunk | delete_hunk | update_hunk
add_hunk: "*** Add File: " filename LF add_line+
delete_hunk: "*** Delete File: " filename LF
update_hunk: "*** Update File: " filename LF change+

filename: /(.+)/
add_line: "+" /(.*)/ LF -> line

change: change_context | change_line
change_context: ("@@" | "@@ " /(.+)/) LF
change_line: ("+" | "-" | " ") /(.*)/ LF

%import common.LF
"#;
