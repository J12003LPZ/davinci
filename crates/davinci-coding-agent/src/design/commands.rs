use super::{error::*, records::*, types::*};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum DesignCommand {
    List {},
    Create {
        input: CreateDesign,
    },
    Open {
        artifact_id: ArtifactId,
    },
    Status {
        artifact_id: ArtifactId,
    },
    Revise {
        artifact_id: ArtifactId,
        brief: String,
        operation_id: OperationId,
    },
    Verify {
        artifact_id: ArtifactId,
    },
    Fork {
        artifact_id: ArtifactId,
        revision: RevisionId,
        operation_id: OperationId,
    },
    Restore {
        artifact_id: ArtifactId,
        revision: RevisionId,
        operation_id: OperationId,
    },
    Export {
        artifact_id: ArtifactId,
        revision: RevisionId,
        format: ExportFormat,
        destination: String,
        artboard_id: Option<ArtboardId>,
        viewport: Option<Viewport>,
    },
    Apply {
        artifact_id: ArtifactId,
        revision: RevisionId,
    },
    Sync {
        path: String,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Source,
    Png,
    Html,
}
fn words(input: &str) -> DesignResult<Vec<String>> {
    let mut result = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut present = false;
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if quote.is_some()
            && ch == '\\'
            && chars
                .peek()
                .is_some_and(|next| Some(*next) == quote || *next == '\\')
        {
            word.push(chars.next().expect("peeked escaped character"));
            continue;
        }
        match (quote, ch) {
            (Some(q), c) if q == c => quote = None,
            (None, '\'' | '"') => {
                quote = Some(ch);
                present = true;
            }
            (None, c) if c.is_whitespace() => {
                if present {
                    result.push(std::mem::take(&mut word));
                    present = false;
                }
            }
            _ => {
                word.push(ch);
                present = true;
            }
        }
    }
    if quote.is_some() {
        return Err(DesignError::InvalidInput("unterminated quote".into()));
    }
    if present {
        result.push(word);
    }
    Ok(result)
}
pub fn parse_design_command(input: &str) -> DesignResult<DesignCommand> {
    if input.len() > 64 * 1024 {
        return Err(DesignError::BudgetExceeded("command length".into()));
    }
    let mut tokens = words(input)?;
    if tokens.first().is_some_and(|s| s == "/design-sync") {
        if tokens.len() != 2 {
            return Err(DesignError::InvalidInput(
                "usage: /design-sync <relative-path>".into(),
            ));
        }
        return Ok(DesignCommand::Sync {
            path: tokens[1].clone(),
        });
    }
    if tokens
        .first()
        .is_some_and(|s| s == "/design" || s == "design")
    {
        tokens.remove(0);
    }
    let Some(command) = tokens.first().map(String::as_str) else {
        return Ok(DesignCommand::List {});
    };
    if command == "--" {
        return create(&tokens[1..], BTreeMap::new());
    }
    let reserved = [
        "new", "list", "open", "status", "revise", "verify", "fork", "restore", "export", "apply",
        "sync",
    ];
    if !reserved.contains(&command) {
        return create(&tokens, BTreeMap::new());
    }
    let mut positional = Vec::new();
    let mut flags = BTreeMap::new();
    let mut i = 1;
    while i < tokens.len() {
        if tokens[i].starts_with("--") {
            let key = tokens[i].as_str();
            let value = tokens
                .get(i + 1)
                .ok_or_else(|| DesignError::InvalidInput("flag needs a value".into()))?;
            let allowed = match command {
                "new" => ["--kind", "--variants"].contains(&key),
                "fork" | "restore" | "apply" => key == "--revision",
                "export" => [
                    "--revision",
                    "--format",
                    "--destination",
                    "--artboard",
                    "--viewport",
                ]
                .contains(&key),
                _ => false,
            };
            if !allowed || value.starts_with("--") || flags.insert(key, value.as_str()).is_some() {
                return Err(DesignError::InvalidInput("unknown or repeated flag".into()));
            }
            i += 2;
        } else {
            positional.push(tokens[i].clone());
            i += 1;
        }
    }
    if command == "new" {
        return create(&positional, flags);
    }
    if command == "list" && positional.is_empty() {
        return Ok(DesignCommand::List {});
    }
    if command == "sync" && positional.len() == 1 {
        return Ok(DesignCommand::Sync {
            path: positional[0].clone(),
        });
    }
    if positional.is_empty() || (command != "revise" && positional.len() != 1) {
        return Err(DesignError::InvalidInput(
            "invalid command arguments".into(),
        ));
    }
    let artifact_id = positional[0].parse()?;
    let revision = || {
        flags
            .get("--revision")
            .ok_or_else(|| DesignError::InvalidInput("--revision is required".into()))
            .and_then(|r| parse_revision_decimal(r))
    };
    Ok(match command {
        "open" => DesignCommand::Open { artifact_id },
        "status" => DesignCommand::Status { artifact_id },
        "verify" => DesignCommand::Verify { artifact_id },
        "revise" if positional.len() > 1 => DesignCommand::Revise {
            artifact_id,
            brief: positional[1..].join(" "),
            operation_id: OperationId::new(),
        },
        "fork" => DesignCommand::Fork {
            artifact_id,
            revision: revision()?,
            operation_id: OperationId::new(),
        },
        "restore" => DesignCommand::Restore {
            artifact_id,
            revision: revision()?,
            operation_id: OperationId::new(),
        },
        "apply" => DesignCommand::Apply {
            artifact_id,
            revision: revision()?,
        },
        "export" => DesignCommand::Export {
            artifact_id,
            revision: revision()?,
            destination: flags
                .get("--destination")
                .ok_or_else(|| {
                    DesignError::InvalidInput(
                        "--destination <new-absolute-directory> is required".into(),
                    )
                })?
                .to_string(),
            artboard_id: flags.get("--artboard").map(|id| id.parse()).transpose()?,
            viewport: flags
                .get("--viewport")
                .map(|value| {
                    let (width, height) = value.split_once('x').ok_or_else(|| {
                        DesignError::InvalidInput("viewport must be WIDTHxHEIGHT".into())
                    })?;
                    let viewport = Viewport {
                        width: width.parse().map_err(|_| {
                            DesignError::InvalidInput("invalid viewport width".into())
                        })?,
                        height: height.parse().map_err(|_| {
                            DesignError::InvalidInput("invalid viewport height".into())
                        })?,
                    };
                    viewport.validate()?;
                    Ok::<_, DesignError>(viewport)
                })
                .transpose()?,
            format: match flags.get("--format").copied().unwrap_or("source") {
                "source" => ExportFormat::Source,
                "png" => ExportFormat::Png,
                "html" => ExportFormat::Html,
                _ => {
                    return Err(DesignError::InvalidInput(
                        "unsupported export format".into(),
                    ))
                }
            },
        },
        _ => {
            return Err(DesignError::InvalidInput(
                "invalid command arguments".into(),
            ))
        }
    })
}
fn create(words: &[String], flags: BTreeMap<&str, &str>) -> DesignResult<DesignCommand> {
    if words.iter().any(|s| s.starts_with("--")) {
        return Err(DesignError::InvalidInput(
            "unknown flag; quote flags inside a brief".into(),
        ));
    }
    let brief = words.join(" ");
    validate_text(&brief, 64 * 1024, "brief")?;
    let kind = match flags.get("--kind").copied().unwrap_or("landing") {
        "landing" => DesignKind::Landing,
        "product" => DesignKind::Product,
        "document" => DesignKind::Document,
        _ => {
            return Err(DesignError::InvalidInput(
                "kind must be landing, product, or document".into(),
            ))
        }
    };
    let variants: u32 = flags
        .get("--variants")
        .copied()
        .unwrap_or("2")
        .parse()
        .map_err(|_| DesignError::InvalidInput("invalid variant count".into()))?;
    if !(1..=3).contains(&variants) {
        return Err(DesignError::InvalidInput("variants must be 1..3".into()));
    }
    let title = brief.chars().take(80).collect();
    Ok(DesignCommand::Create {
        input: CreateDesign {
            title,
            brief,
            kind,
            operation_id: OperationId::new(),
            variants,
        },
    })
}
