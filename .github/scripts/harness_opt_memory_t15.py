from pathlib import Path
import re
import sys

VECTOR = Path('crates/davinci-coding-agent/src/native_extensions/vector_memory.rs')
ECOSYSTEM = Path('crates/davinci-coding-agent/src/native_extensions/ecosystem/mod.rs')
DECISION = Path('docs/superpowers/plans/2026-09-15-memory-projection-decision.md')

TEST_MARKER = 'mod phase3_t15_projection_regressions'
TEST_MODULE = r'''

#[cfg(test)]
mod phase3_t15_projection_regressions {
    use super::*;

    #[test]
    fn phase3_t15_projection_profile_is_explicit_local() {
        assert_eq!(MEMORY_PROJECTION_PROFILE, "local");
        let dir = tempfile::tempdir().unwrap();
        let memory = VectorMemory::with_config(
            dir.path().to_path_buf(),
            VectorMemoryConfig::default(),
        );
        assert!(!memory.remote_projection_enabled());
    }

    #[test]
    fn phase3_t15_embedding_identity_changes_with_model_or_dimensions() {
        let base = VectorMemoryConfig::default();
        let base_identity = EmbeddingIdentity::from_config(&base);

        let mut changed_model = base.clone();
        changed_model.embedding_model = "replacement-model".into();
        assert_ne!(base_identity, EmbeddingIdentity::from_config(&changed_model));

        let mut changed_dimensions = base;
        changed_dimensions.embedding_dimensions = 384;
        assert_ne!(
            base_identity,
            EmbeddingIdentity::from_config(&changed_dimensions)
        );
    }

    #[test]
    fn phase3_t15_legacy_or_changed_embedding_is_incompatible() {
        let config = VectorMemoryConfig::default();
        let current = EmbeddingIdentity::from_config(&config);
        assert!(!embedding_identity_compatible(None, &current));
        assert!(embedding_identity_compatible(Some(&current), &current));

        let mut other = current.clone();
        other.model = "other-model".into();
        assert!(!embedding_identity_compatible(Some(&other), &current));
    }
}
'''

DECISION_TEXT = '''# Memory vector projection decision — 2026-09-15

## Decision

Use the **local bounded retrieval/index as the production projection profile** for Phase 3. Qdrant remains a dormant compatibility surface and is not written on the normal indexing path.

## Break-even result

The inspected production path had local lexical+dense retrieval, while Qdrant was **write-only**: records were upserted but no Qdrant query participated in retrieval. Therefore there is no representative corpus size at which the existing remote projection can improve retrieval quality or latency; it only adds network and maintenance work. Claiming a measured remote break-even from that path would be misleading.

The local path is already bounded by `candidateLimit` and `maxIndexChunks`, preserves exact identifiers, and has labeled retrieval evaluation. This makes local the only projection with a demonstrated end-to-end retrieval benefit today.

## Reconsideration gate

Reconsider Qdrant only after a real namespace-filtered remote query path exists and the E4 gate from the engineering plan is measured on fixed corpora. Promotion requires **at least 30% better retrieval p95 at equal labeled quality**, with service-outage fallback, namespace isolation, tombstone exclusion, projection-lag accounting, collection/version validation, and model-change rebuild tests passing.

## Compatibility and rebuild policy

Persisted embeddings carry an identity containing model, dimensions, and the embedding-prefix revision. Legacy or mismatched embeddings are excluded from dense scoring. The authoritative local records remain available to lexical retrieval, and the local rebuild helper can regenerate bounded batches under the current identity. This permits safe rollback without deleting authoritative memory.
'''


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f'{label}: expected one anchor, found {count}')
    return text.replace(old, new, 1)


def add_tests() -> None:
    text = VECTOR.read_text()
    if TEST_MARKER not in text:
        VECTOR.write_text(text + TEST_MODULE)


def implement() -> None:
    text = VECTOR.read_text()

    if 'pub const MEMORY_PROJECTION_PROFILE' not in text:
        text = replace_once(
            text,
            'pub const SEARCH_LIMIT_CAP: usize = 20;\n',
            'pub const SEARCH_LIMIT_CAP: usize = 20;\n/// Phase 3 chooses the bounded local index as the production projection.\n/// Qdrant configuration remains readable for rollback/experiments but normal\n/// indexing does not perform remote writes until a remote query path clears\n/// the documented break-even gate.\npub const MEMORY_PROJECTION_PROFILE: &str = "local";\nconst EMBEDDING_PREFIX_REVISION: u32 = 1;\n',
            'projection profile constants',
        )

    if 'pub struct EmbeddingIdentity' not in text:
        anchor = '#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]\n#[serde(rename_all = "camelCase")]\npub struct MemoryRecord {'
        identity = '''#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]\n#[serde(rename_all = "camelCase")]\npub struct EmbeddingIdentity {\n    pub model: String,\n    pub dimensions: usize,\n    pub prefix_revision: u32,\n}\n\nimpl EmbeddingIdentity {\n    pub fn from_config(config: &VectorMemoryConfig) -> Self {\n        Self {\n            model: config.embedding_model.clone(),\n            dimensions: config.embedding_dimensions,\n            prefix_revision: EMBEDDING_PREFIX_REVISION,\n        }\n    }\n}\n\nfn embedding_identity_compatible(\n    stored: Option<&EmbeddingIdentity>,\n    current: &EmbeddingIdentity,\n) -> bool {\n    stored == Some(current)\n}\n\n#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]\n#[serde(rename_all = "camelCase")]\npub struct MemoryRecord {'''
        text = replace_once(text, anchor, identity, 'embedding identity type')

    if 'pub embedding_identity: Option<EmbeddingIdentity>' not in text:
        text = replace_once(
            text,
            '    pub embedding: Option<Vec<f32>>,\n',
            '    pub embedding: Option<Vec<f32>>,\n    #[serde(default, skip_serializing_if = "Option::is_none")]\n    pub embedding_identity: Option<EmbeddingIdentity>,\n',
            'record embedding identity field',
        )

    # Every explicit fresh record in this module begins without an embedding identity.
    text = re.sub(
        r'(\n\s*embedding: None,\n)(?!\s*embedding_identity:)',
        r'\1                embedding_identity: None,\n',
        text,
    )

    # Persist compatibility identity whenever a new document vector is stored.
    if 'stored.embedding_identity = Some(embedding_identity.clone());' not in text:
        old = '''                    for (record, embedding) in inserted_records.iter().zip(embeddings) {\n                        if let Some(stored) =\n                            self.records.iter_mut().find(|item| item.id == record.id)\n                        {\n                            stored.embedding = Some(embedding);\n                        }\n                    }'''
        new = '''                    let embedding_identity = EmbeddingIdentity::from_config(&self.config);\n                    for (record, embedding) in inserted_records.iter().zip(embeddings) {\n                        if let Some(stored) =\n                            self.records.iter_mut().find(|item| item.id == record.id)\n                        {\n                            stored.embedding = Some(embedding);\n                            stored.embedding_identity = Some(embedding_identity.clone());\n                        }\n                    }'''
        text = replace_once(text, old, new, 'index embedding identity persistence')

    if 'stored.embedding_identity = Some(EmbeddingIdentity::from_config(&self.config));' not in text:
        old = '''                    if let Some(stored) = self.records.iter_mut().find(|item| item.id == id) {\n                        stored.embedding = Some(emb.clone());\n                    }'''
        new = '''                    if let Some(stored) = self.records.iter_mut().find(|item| item.id == id) {\n                        stored.embedding = Some(emb.clone());\n                        stored.embedding_identity =\n                            Some(EmbeddingIdentity::from_config(&self.config));\n                    }'''
        text = replace_once(text, old, new, 'learning embedding identity persistence')

    # If the legacy remote-upsert struct update remains, keep its metadata correct.
    text = text.replace(
        '                        embedding: Some(emb),\n                        ..record',
        '                        embedding: Some(emb),\n                        embedding_identity: Some(EmbeddingIdentity::from_config(&self.config)),\n                        ..record',
    )

    # Add local projection helpers before status().
    if 'pub fn remote_projection_enabled(&self) -> bool' not in text:
        status_anchor = '    pub fn status(&self) -> Value {'
        helpers = '''    /// The selected Phase 3 projection is deliberately local. Remote write\n    /// amplification stays disabled until a filtered remote retrieval path\n    /// demonstrates the documented quality/latency break-even.\n    pub fn remote_projection_enabled(&self) -> bool {\n        false\n    }\n\n    fn current_embedding_identity(&self) -> EmbeddingIdentity {\n        EmbeddingIdentity::from_config(&self.config)\n    }\n\n    fn drop_incompatible_embeddings(&mut self) -> usize {\n        let current = self.current_embedding_identity();\n        let mut dropped = 0usize;\n        for record in &mut self.records {\n            if record.embedding.is_some()\n                && !embedding_identity_compatible(record.embedding_identity.as_ref(), &current)\n            {\n                record.embedding = None;\n                record.embedding_identity = None;\n                dropped += 1;\n            }\n        }\n        dropped\n    }\n\n    fn local_projection_lag(&self) -> usize {\n        let current = self.current_embedding_identity();\n        self.records\n            .iter()\n            .filter(|record| {\n                !self.tombstones.contains(&record.id)\n                    && !self.supersessions.contains_key(&record.id)\n                    && (record.embedding.is_none()\n                        || !embedding_identity_compatible(\n                            record.embedding_identity.as_ref(),\n                            &current,\n                        ))\n            })\n            .count()\n    }\n\n    /// Rebuild a bounded batch of missing or incompatible local embeddings.\n    /// Authoritative records remain readable through lexical retrieval if the\n    /// embedding service is unavailable. Tombstoned/superseded records are\n    /// never projected.\n    #[allow(dead_code)]\n    pub fn rebuild_local_embeddings(&mut self, limit: usize) -> Result<usize, ToolError> {\n        self.drop_incompatible_embeddings();\n        let pending = self\n            .records\n            .iter()\n            .filter(|record| {\n                !self.tombstones.contains(&record.id)\n                    && !self.supersessions.contains_key(&record.id)\n                    && record.embedding.is_none()\n            })\n            .take(limit.max(1))\n            .map(|record| (record.id.clone(), record.text.clone()))\n            .collect::<Vec<_>>();\n        if pending.is_empty() {\n            return Ok(0);\n        }\n        let texts = pending.iter().map(|(_, text)| text.clone()).collect::<Vec<_>>();\n        let embeddings = self.embed_documents(&texts)?;\n        if embeddings.len() != pending.len() {\n            return Err(ToolError::Failed(\n                "embedding rebuild response count does not match request".into(),\n            ));\n        }\n        let identity = self.current_embedding_identity();\n        let mut updated = 0usize;\n        for ((id, _), embedding) in pending.into_iter().zip(embeddings) {\n            if let Some(record) = self.records.iter_mut().find(|record| record.id == id) {\n                record.embedding = Some(embedding);\n                record.embedding_identity = Some(identity.clone());\n                updated += 1;\n            }\n        }\n        if updated > 0 {\n            self.persist_local()?;\n        }\n        Ok(updated)\n    }\n\n'''
        text = replace_once(text, status_anchor, helpers + status_anchor, 'projection helper insertion')

    # Dense retrieval is read-only. Ignore legacy/model-mismatched vectors in-place
    # instead of mutating the authoritative store during a search.
    if 'let embedding_identity = self.current_embedding_identity();' not in text[text.find('pub fn search_scoped'):text.find('pub fn search_scoped') + 5000]:
        old = '''        let query_embedding = (self.dense_available()\n            && candidates.iter().any(|record| record.embedding.is_some()))'''
        new = '''        let embedding_identity = self.current_embedding_identity();\n        let query_embedding = (self.dense_available()\n            && candidates.iter().any(|record| {\n                record.embedding.is_some()\n                    && embedding_identity_compatible(\n                        record.embedding_identity.as_ref(),\n                        &embedding_identity,\n                    )\n            }))'''
        text = replace_once(text, old, new, 'compatible dense query gate')

        old = '''                    .filter_map(|(index, record)| {\n                        let embedding = record.embedding.as_ref()?;\n                        if embedding.len() != query_vector.len() {'''
        new = '''                    .filter_map(|(index, record)| {\n                        if !embedding_identity_compatible(\n                            record.embedding_identity.as_ref(),\n                            &embedding_identity,\n                        ) {\n                            return None;\n                        }\n                        let embedding = record.embedding.as_ref()?;\n                        if embedding.len() != query_vector.len() {'''
        text = replace_once(text, old, new, 'compatible dense ranking filter')

    # Disable automatic remote projection writes while keeping the helper for rollback/experiments.
    text = text.replace(
        '                    let _ = self.upsert_remote(&embedded);',
        '                    if self.remote_projection_enabled() {\n                        let _ = self.upsert_remote(&embedded);\n                    }',
    )
    text = text.replace(
        '''                    let _ = self.upsert_remote(&[MemoryRecord {\n                        embedding: Some(emb),\n                        embedding_identity: Some(EmbeddingIdentity::from_config(&self.config)),\n                        ..record\n                    }]);''',
        '''                    if self.remote_projection_enabled() {\n                        let _ = self.upsert_remote(&[MemoryRecord {\n                            embedding: Some(emb),\n                            embedding_identity: Some(EmbeddingIdentity::from_config(&self.config)),\n                            ..record\n                        }]);\n                    }''',
    )

    # Surface the decision and measurable projection lag.
    if '"projectionProfile": MEMORY_PROJECTION_PROFILE' not in text:
        text = replace_once(
            text,
            '            "denseAvailable": self.dense_available(),\n',
            '            "denseAvailable": self.dense_available(),\n            "projectionProfile": MEMORY_PROJECTION_PROFILE,\n            "remoteProjectionEnabled": self.remote_projection_enabled(),\n            "projectionLag": self.local_projection_lag(),\n',
            'projection status fields',
        )

    VECTOR.write_text(text)

    # Ecosystem fixtures construct MemoryRecord directly. Keep those source-level
    # initializers backward-compatible with the new persisted optional field.
    ecosystem = ECOSYSTEM.read_text()
    ecosystem = re.sub(
        r'(\n\s*embedding: None,\n)(?!\s*embedding_identity:)',
        lambda match: match.group(1) + match.group(1).split('embedding:')[0] + 'embedding_identity: None,\n',
        ecosystem,
    )
    ECOSYSTEM.write_text(ecosystem)

    DECISION.parent.mkdir(parents=True, exist_ok=True)
    DECISION.write_text(DECISION_TEXT)


mode = sys.argv[1] if len(sys.argv) > 1 else ''
if mode == 'tests':
    add_tests()
elif mode == 'impl':
    implement()
else:
    raise SystemExit('usage: harness_opt_memory_t15.py tests|impl')
