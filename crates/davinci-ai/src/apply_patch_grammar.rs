//! Lark grammar for the freeform `apply_patch` tool.
//! Mirrors the patch forms accepted by DaVinci's Codex patch parser.

pub const APPLY_PATCH_LARK: &str = r#"start: begin_patch hunk+ end_patch
begin_patch: "*** Begin Patch" LF
end_patch: "*** End Patch" LF?

hunk: add_hunk | delete_hunk | update_hunk
add_hunk: "*** Add File: " filename LF add_line* end_of_file?
delete_hunk: "*** Delete File: " filename LF
update_hunk: "*** Update File: " filename LF update_section+
update_section: change_context change_line+ end_of_file?

filename: LINE_TEXT
add_line: "+" LINE_TEXT? LF -> line
end_of_file: "*** End of File" LF

change_context: ("@@" | "@@ " LINE_TEXT) LF
change_line: ("+" | "-" | " ") LINE_TEXT? LF | LF

LINE_TEXT: /[^\r\n]+/
LF: /\r?\n/
"#;
