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
    /// Choose one concept (or `none`); later revisions touch only that concept.
    Select {
        artifact_id: ArtifactId,
        variant_id: Option<VariantId>,
    },
    /// Try the failed phase of a blocked generation again from its checkpoint.
    Resume {
        artifact_id: ArtifactId,
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
/// Split a command line into words. A quote opens a quoted section only at
/// the start of a word and closes it only at the end of one, so an apostrophe
/// inside prose ("Julien's team", 5" screen) is an ordinary character.
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
            (Some(q), c) if q == c && chars.peek().is_none_or(|next| next.is_whitespace()) => {
                quote = None
            }
            (None, '\'' | '"') if !present => {
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
    let Some(command) = tokens.first().cloned() else {
        return Ok(DesignCommand::List {});
    };
    let command = command.as_str();
    if command == "--" {
        return create(&tokens[1..], BTreeMap::new());
    }
    let reserved = [
        "new", "list", "open", "status", "revise", "resume", "select", "verify", "fork", "restore",
        "export", "apply", "sync",
    ];
    if !reserved.contains(&command) {
        return create(&tokens, BTreeMap::new());
    }
    // A reserved word is always a verb. When what follows does not fit it
    // (no argument, an empty one, a mistyped id) that is a usage error, never
    // a brief: a typo must not start a paid generation. A brief that begins
    // with one of these words goes after `new` or `--`.
    // Ids may be typed in capitals; a brief's own words are never changed.
    let id_tokens = if matches!(command, "resume" | "select") {
        2
    } else {
        1
    };
    for token in tokens.iter_mut().skip(1).take(id_tokens) {
        *token = normalize_id(token);
    }
    let rest = &tokens[1..];
    let fits = match command {
        "new" => true,
        "list" => rest.is_empty(),
        "sync" => rest.len() == 1 && !rest[0].starts_with("--"),
        _ => rest
            .first()
            .is_some_and(|first| first.parse::<ArtifactId>().is_ok()),
    };
    if !fits {
        return Err(DesignError::InvalidInput(usage(command)));
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
    let expected = if matches!(command, "resume" | "select") {
        2
    } else {
        1
    };
    if positional.is_empty() || (command != "revise" && positional.len() != expected) {
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
        "select" => DesignCommand::Select {
            artifact_id,
            variant_id: match positional[1].as_str() {
                "none" => None,
                other => Some(other.parse()?),
            },
        },
        "resume" => DesignCommand::Resume {
            artifact_id,
            operation_id: positional[1].parse()?,
        },
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
/// A UUID typed in capitals is the same id; everything else is left alone.
fn normalize_id(token: &str) -> String {
    let uuid_shaped =
        token.len() == 36 && token.chars().all(|ch| ch.is_ascii_hexdigit() || ch == '-');
    if uuid_shaped {
        token.to_ascii_lowercase()
    } else {
        token.to_string()
    }
}

fn usage(command: &str) -> String {
    let line = match command {
        "list" => "/design list",
        "sync" => "/design sync <relative-path>",
        "open" => "/design open <artifact-id>",
        "status" => "/design status <artifact-id>",
        "verify" => "/design verify <artifact-id>",
        "revise" => "/design revise <artifact-id> <what to change>",
        "resume" => "/design resume <artifact-id> <checkpoint-id>",
        "select" => "/design select <artifact-id> <concept-id|none>",
        "fork" => "/design fork <artifact-id> --revision <n>",
        "restore" => "/design restore <artifact-id> --revision <n>",
        "apply" => "/design apply <artifact-id> --revision <n>",
        "export" => "/design export <artifact-id> --revision <n> --destination <new-absolute-directory> [--format source|png|html] [--artboard <id>] [--viewport WxH]",
        _ => "/design new \"<brief>\"",
    };
    format!(
        "usage: {line} (the id is the UUID from /design list; to start a brief that begins with \"{command}\", write /design new \"{command} ...\")"
    )
}

fn create(words: &[String], flags: BTreeMap<&str, &str>) -> DesignResult<DesignCommand> {
    if words.iter().any(|s| s.starts_with("--")) {
        return Err(DesignError::InvalidInput(
            "unknown flag; quote flags inside a brief".into(),
        ));
    }
    let brief = words.join(" ");
    validate_text(&brief, 64 * 1024, "brief")?;
    let explicit_kind = flags
        .get("--kind")
        .map(|kind| match *kind {
            "landing" => Ok(DesignKind::Landing),
            "product" => Ok(DesignKind::Product),
            "document" => Ok(DesignKind::Document),
            _ => Err(DesignError::InvalidInput(
                "kind must be landing, product, or document".into(),
            )),
        })
        .transpose()?;
    let variants: u32 = match flags.get("--variants") {
        Some(value) => value
            .parse()
            .map_err(|_| DesignError::InvalidInput("invalid variant count".into()))?,
        None => super::intent::DEFAULT_VARIANTS,
    };
    if !(1..=3).contains(&variants) {
        return Err(DesignError::InvalidInput("variants must be 1..3".into()));
    }
    let resolved = super::intent::DesignIntent {
        brief,
        explicit_kind,
        variants,
    }
    .resolve();
    let title = resolved.brief.chars().take(80).collect();
    Ok(DesignCommand::Create {
        input: CreateDesign {
            title,
            brief: resolved.brief,
            kind: resolved.kind,
            operation_id: OperationId::new(),
            variants: resolved.variants,
        },
    })
}
