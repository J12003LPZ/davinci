use super::{admission::*, error::*, records::*, store::*, types::*};
use davinci_session::JsonlSession;
use std::collections::BTreeSet;

pub fn validate_bindings(write: &RevisionWrite) -> DesignResult<()> {
    let mut nodes = BTreeSet::new();
    for binding in &write.bindings {
        if let BindingConstraint::Number { min, max } = &binding.constraint {
            const SAFE: i64 = 9_007_199_254_740_991;
            if *min < -SAFE || *max > SAFE || min > max {
                return Err(DesignError::InvalidInput(
                    "numeric binding must fit exact JavaScript integers".into(),
                ));
            }
        }
        let source = write
            .sources
            .files
            .get(&binding.source_file)
            .ok_or_else(|| DesignError::InvalidInput("binding source missing".into()))?;
        // Each rule says what failed: a generation run hands this message
        // back to the model to repair.
        let pointer_ok = binding.pointer.starts_with('/')
            && binding.pointer.len() <= 1024
            && !binding.pointer.split('/').any(|p| {
                let decoded = p.replace("~1", "/").replace("~0", "~");
                ["__proto__", "prototype", "constructor"].contains(&decoded.as_str())
                    || p.as_bytes()
                        .windows(2)
                        .any(|pair| pair[0] == b'~' && pair[1] != b'0' && pair[1] != b'1')
                    || p.ends_with('~')
            });
        let artboard_ok = write
            .variants
            .iter()
            .flat_map(|v| &v.artboards)
            .any(|b| b.id == binding.artboard_id);
        let failure = if !nodes.insert(binding.node_id) {
            Some("node_id is used by another binding".to_string())
        } else if !binding.source_file.ends_with(".json") {
            Some(format!(
                "source_file {} must be a JSON file in files",
                binding.source_file
            ))
        } else if binding.source_hash != source.sha256 {
            Some("source_hash does not match the source file".to_string())
        } else if !pointer_ok {
            Some(format!(
                "pointer {} must be a JSON Pointer such as /hero/title",
                binding.pointer
            ))
        } else if !artboard_ok {
            Some(format!(
                "artboard_id {} is not an artboard in variants",
                binding.artboard_id
            ))
        } else {
            None
        };
        if let Some(failure) = failure {
            return Err(DesignError::InvalidInput(format!(
                "invalid declared binding: {failure}"
            )));
        }
        let affected: BTreeSet<_> = write
            .bindings
            .iter()
            .filter(|b| b.source_file == binding.source_file && b.pointer == binding.pointer)
            .map(|b| b.node_id)
            .collect();
        if affected != binding.affected_nodes.iter().copied().collect()
            || affected.len() != binding.affected_nodes.len()
        {
            return Err(DesignError::InvalidInput(
                "binding must disclose every affected node".into(),
            ));
        }
    }
    Ok(())
}
pub(crate) fn validate_value(
    binding: &EditableBinding,
    value: &serde_json::Value,
) -> DesignResult<()> {
    let valid = match &binding.constraint {
        BindingConstraint::Text { max_bytes } => {
            *max_bytes <= 64 * 1024
                && value
                    .as_str()
                    .is_some_and(|s| s.len() as u64 <= u64::from(*max_bytes) && !s.contains('\0'))
        }
        BindingConstraint::Enum { values } => {
            values.len() <= 128
                && value
                    .as_str()
                    .is_some_and(|s| values.iter().any(|v| v == s))
        }
        BindingConstraint::Number { min, max } => {
            min <= max && value.as_i64().is_some_and(|n| n >= *min && n <= *max)
        }
        BindingConstraint::Color {} => value.as_str().is_some_and(|s| {
            (s.len() == 7 || s.len() == 9)
                && s.starts_with('#')
                && s.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
        }),
        BindingConstraint::LocalAsset { paths } => value
            .as_str()
            .is_some_and(|s| validate_path(s).is_ok() && paths.iter().any(|p| p == s)),
    };
    if valid {
        Ok(())
    } else {
        Err(DesignError::InvalidInput(
            "value violates the declared property constraint".into(),
        ))
    }
}
pub fn apply_edit(
    store: &DesignStore,
    ctx: &AuthorizedDesignContext,
    session: &mut JsonlSession,
    edit: DesignEdit,
) -> DesignResult<DesignRevision> {
    ctx.check_session(session)?;
    ctx.check("design_patch", &serde_json::to_value(&edit)?)?;
    let current = store.read_revision(ctx, session, edit.artifact_id, edit.expected_revision)?;
    let binding = current
        .bindings
        .iter()
        .find(|b| b.node_id == edit.node_id)
        .ok_or_else(|| DesignError::NotFound("editable node missing".into()))?;
    if digest(binding)? != edit.expected_binding_hash {
        return Err(DesignError::Conflict(
            "binding changed; refresh inspector".into(),
        ));
    }
    if binding
        .affected_nodes
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        != edit.confirmed_affected_nodes.iter().copied().collect()
        || binding.affected_nodes.len() != edit.confirmed_affected_nodes.len()
    {
        return Err(DesignError::Denied(
            "confirm all affected nodes before changing a shared value".into(),
        ));
    }
    validate_value(binding, &edit.value)?;
    let mut files = store.read_sources(ctx, session, edit.artifact_id, edit.expected_revision)?;
    let text = files
        .get(&binding.source_file)
        .ok_or_else(|| DesignError::CorruptArtifact("binding file missing".into()))?;
    let mut content: serde_json::Value = serde_json::from_str(text)?;
    let slot = content
        .pointer_mut(&binding.pointer)
        .ok_or_else(|| DesignError::Conflict("binding path no longer exists".into()))?;
    if slot.is_array() || slot.is_object() {
        return Err(DesignError::InvalidInput(
            "only declared scalar properties are editable".into(),
        ));
    }
    *slot = edit.value;
    files.insert(
        binding.source_file.clone(),
        serde_json::to_string_pretty(&content)?,
    );
    let sources =
        store.store_sources(ctx, session, &files, current.sources.entry_points.clone())?;
    let bindings = current
        .bindings
        .iter()
        .map(|b| EditableBinding {
            source_hash: sources.files[&b.source_file].sha256.clone(),
            ..b.clone()
        })
        .collect();
    store.commit_revision(
        ctx,
        session,
        RevisionWrite {
            artifact_id: edit.artifact_id,
            expected_revision: edit.expected_revision,
            operation_id: edit.operation_id,
            sources,
            variants: current.variants,
            bindings,
            assets: current.assets,
            profile_refs: current.profile_refs,
            system_snapshot: current.system_snapshot,
        },
    )
}
