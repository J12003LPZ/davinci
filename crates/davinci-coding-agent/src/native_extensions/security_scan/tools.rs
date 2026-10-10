//! Snapshot-only worker tools. No fallback into builtins, MCP, or extensions.

use super::snapshot::Snapshot;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReadInput {
    path: String,
    start_line: usize,
    end_line: usize,
    #[serde(default = "super::validation::default_side")]
    snapshot_side: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SearchInput {
    query: String,
    offset: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListInput {
    offset: usize,
}

pub(super) fn inventory_entry(
    snapshot: &Snapshot,
    side: &str,
    path: &str,
    file: &super::snapshot::SourceFile,
) -> Result<Value, String> {
    let policy = super::policy::resolve(snapshot, path, side)?;
    let mut entry = json!({"path":path,"snapshotSide":side,"contentHash":file.hash,
        "lines":file.text.lines().count(),"policy":policy,"scope":if snapshot.is_target(path,side) {"target"} else {"supporting"}});
    if ["index-base", "index-ours", "index-theirs"].contains(&side) {
        entry["indexIdentity"] = json!(snapshot.conflict_source(path, side)?.identity);
    }
    Ok(entry)
}

pub fn execute(snapshot: &Snapshot, name: &str, args: Value) -> Result<Value, String> {
    match name {
        "sec_source_list" => {
            let input: ListInput =
                serde_json::from_value(args).map_err(|_| "invalid inventory input")?;
            let total = snapshot.source_count();
            let entries: Vec<_> = snapshot
                .sources()
                .skip(input.offset)
                .take(100)
                .map(|(side, path, file)| inventory_entry(snapshot, side, path, file))
                .collect::<Result<_, _>>()?;
            Ok(
                json!({"snapshotId":snapshot.id,"entries":entries,"total":total,"nextOffset":(input.offset.saturating_add(100)<total).then_some(input.offset.saturating_add(100))}),
            )
        }
        "sec_source_read" => {
            let input: ReadInput =
                serde_json::from_value(args).map_err(|_| "invalid source read input")?;
            let raw = if input.snapshot_side == snapshot.current_side() {
                snapshot.read(&input.path, input.start_line, input.end_line)?
            } else {
                snapshot.read_side(
                    &input.path,
                    input.start_line,
                    input.end_line,
                    &input.snapshot_side,
                )?
            };
            // A window can start inside a private key or a multi-line quoted
            // secret, after the line that identifies it. Mask the whole file
            // (line count is kept, computed once per file), then take the
            // same lines.
            let file = snapshot.file(&input.path, &input.snapshot_side)?;
            let masked = snapshot.masked_text(&input.path, &input.snapshot_side)?;
            let window = masked
                .lines()
                .skip(input.start_line - 1)
                .take(input.end_line + 1 - input.start_line)
                .collect::<Vec<_>>()
                .join("\n");
            let text = super::redaction::text(&window);
            Ok(
                json!({"snapshotId":snapshot.id, "path": input.path, "contentHash": file.hash,"snapshotSide":input.snapshot_side,
                "startLine":input.start_line,"endLine":input.end_line,"redacted":text != raw,"text":text,
                "scope":if snapshot.is_target(&input.path,&input.snapshot_side) {"target"} else {"supporting"}}),
            )
        }
        "sec_source_search" => {
            let input: SearchInput =
                serde_json::from_value(args).map_err(|_| "invalid source search input")?;
            if input.query.is_empty() || input.query.len() > 256 {
                return Err("search query requires 1..256 bytes".into());
            }
            let mut matches = Vec::new();
            let mut encountered = 0usize;
            let mut more = false;
            'files: for (side, path, file) in snapshot.sources() {
                for (index, line) in file.text.lines().enumerate() {
                    if line.contains(&input.query) {
                        encountered += 1;
                        if encountered <= input.offset {
                            continue;
                        }
                        if matches.len() == 50 {
                            more = true;
                            break 'files;
                        }
                        // Search returns locations only. Exact evidence is retrieved
                        // through the bounded source read, not silently truncated.
                        matches.push(json!({"path":path,"line":index+1,"contentHash":file.hash,"snapshotSide":side,
                            "scope":if snapshot.is_target(path,side) {"target"} else {"supporting"}}));
                    }
                }
            }
            Ok(
                json!({"snapshotId":snapshot.id,"matches":matches,"nextOffset":more.then_some(input.offset.saturating_add(50))}),
            )
        }
        _ => Err("tool denied: security workers only have snapshot source read and search".into()),
    }
}

pub fn specs() -> Vec<davinci_ai::ToolSpec> {
    vec![
        davinci_ai::ToolSpec { name: "sec_source_list".into(), description: "Page immutable source inventory, including both revision sides; at most 100 entries.".into(),
            parameters: json!({"type":"object","additionalProperties":false,"required":["offset"],"properties":{"offset":{"type":"integer","minimum":0}}}), constrained_sampling: None },
        davinci_ai::ToolSpec { name:"sec_source_read".into(), description:"Read 1..200 exact lines from the authorized immutable snapshot, at most 64 KiB.".into(),
            parameters:json!({"type":"object","additionalProperties":false,"required":["path","startLine","endLine"],"properties":{"path":{"type":"string"},"startLine":{"type":"integer","minimum":1},"endLine":{"type":"integer","minimum":1},"snapshotSide":{"type":"string","enum":["worktree","base","head","index-base","index-ours","index-theirs"]}}}), constrained_sampling:None },
        davinci_ai::ToolSpec { name:"sec_source_search".into(), description:"Find literal text in the authorized snapshot. Returns at most 50 locations with explicit pagination.".into(),
            parameters:json!({"type":"object","additionalProperties":false,"required":["query","offset"],"properties":{"query":{"type":"string","minLength":1,"maxLength":256},"offset":{"type":"integer","minimum":0}}}), constrained_sampling:None },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_worker_denies_unadvertised_and_aliased_tools() {
        let snapshot = Snapshot {
            id: "fixture".into(),
            files: Default::default(),
            skipped: vec![],
            bytes: 0,
            ..Default::default()
        };
        for name in [
            "read",
            "bash",
            "exec_command",
            "agent",
            "mcp_read",
            "sec_scan_complete",
            "sec_candidates_validate",
        ] {
            assert!(execute(&snapshot, name, json!({})).is_err());
        }
        assert_eq!(specs().len(), 3);
        assert!(execute(
            &snapshot,
            "sec_source_read",
            json!({"path":"../outside","startLine":1,"endLine":1})
        )
        .is_err());
        assert!(execute(
            &snapshot,
            "sec_source_search",
            json!({"query":"text","offset":0,"network":true})
        )
        .is_err());
    }

    #[test]
    fn security_worker_cannot_read_other_scan_or_host_files() {
        let snapshot = Snapshot {
            id: "fixture".into(),
            files: [(
                "src.rs".into(),
                super::super::snapshot::SourceFile {
                    hash: super::super::sha256_hex(b"fn main() {}\n"),
                    text: "fn main() {}\n".into(),
                },
            )]
            .into(),
            skipped: vec![],
            bytes: 12,
            ..Snapshot::default()
        };
        for path in [
            "../outside.rs",
            "..\\outside.rs",
            "/etc/passwd",
            "C:\\Windows\\win.ini",
            "\\\\unc\\share\\file",
            "report-1.json",
            "security-scans/other/findings.json",
        ] {
            assert!(
                execute(
                    &snapshot,
                    "sec_source_read",
                    json!({"path":path,"startLine":1,"endLine":1})
                )
                .is_err(),
                "{path}"
            );
        }
        let listed = execute(&snapshot, "sec_source_list", json!({"offset":0})).unwrap();
        let entries = listed["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["path"], "src.rs");
    }

    #[test]
    fn source_reads_never_return_private_key_body_lines() {
        let body = "MIIEfixture0FIXTURE1fixture2FIXTURE3fixture4FIXTURE5abcd";
        let text = format!(
            "fn before() {{}}\nconst KEY: &str = \"\\\n-----BEGIN RSA PRIVATE KEY-----\n{body}\n{body}\n-----END RSA PRIVATE KEY-----\";\nfn after() {{}}\n"
        );
        let snapshot = Snapshot {
            id: "fixture".into(),
            files: [(
                "key.rs".into(),
                super::super::snapshot::SourceFile {
                    hash: super::super::sha256_hex(text.as_bytes()),
                    text: text.clone(),
                },
            )]
            .into(),
            ..Snapshot::default()
        };
        // Whole file, a window starting inside the body, and a single body line.
        for (start, end) in [(1, 7), (4, 5), (5, 5), (4, 7)] {
            let read = execute(
                &snapshot,
                "sec_source_read",
                json!({"path":"key.rs","startLine":start,"endLine":end}),
            )
            .unwrap();
            let returned = read["text"].as_str().unwrap();
            assert!(!returned.contains(body), "{start}-{end}: {returned}");
            assert_eq!(returned.lines().count(), end + 1 - start, "{returned}");
            assert_eq!(read["redacted"], true);
        }
        let tail = execute(
            &snapshot,
            "sec_source_read",
            json!({"path":"key.rs","startLine":7,"endLine":7}),
        )
        .unwrap();
        assert_eq!(tail["text"], "fn after() {}");
    }

    #[test]
    fn source_reads_starting_inside_a_quoted_secret_stay_masked() {
        let text = "fn before() {}\nlet password = \"fixture first\nfixture second\nfixture third\";\nfn after() {}\n";
        let snapshot = Snapshot {
            id: "fixture".into(),
            files: [(
                "cfg.rs".into(),
                super::super::snapshot::SourceFile {
                    hash: super::super::sha256_hex(text.as_bytes()),
                    text: text.into(),
                },
            )]
            .into(),
            ..Snapshot::default()
        };
        for (start, end) in [(1, 5), (3, 3), (3, 4), (4, 5)] {
            let read = execute(
                &snapshot,
                "sec_source_read",
                json!({"path":"cfg.rs","startLine":start,"endLine":end}),
            )
            .unwrap();
            let returned = read["text"].as_str().unwrap();
            assert!(!returned.contains("fixture"), "{start}-{end}: {returned}");
        }
        let tail = execute(
            &snapshot,
            "sec_source_read",
            json!({"path":"cfg.rs","startLine":5,"endLine":5}),
        )
        .unwrap();
        assert_eq!(tail["text"], "fn after() {}");
    }
}
