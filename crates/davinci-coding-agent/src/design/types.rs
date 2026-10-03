pub use super::blob::ArtifactRef;
use super::error::{DesignError, DesignResult};
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};
use uuid::Uuid;

macro_rules! opaque_id {
    ($($name:ident),+ $(,)?) => {$(
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);
        impl $name {
            pub fn new() -> Self { Self(Uuid::new_v4()) }
        }
        impl Default for $name { fn default() -> Self { Self::new() } }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { self.0.fmt(f) }
        }
        impl std::str::FromStr for $name {
            type Err = DesignError;
            fn from_str(value: &str) -> DesignResult<Self> {
                let id = Uuid::parse_str(value).map_err(|_| DesignError::InvalidInput("invalid UUID".into()))?;
                if id.is_nil() || id.to_string() != value {
                    return Err(DesignError::InvalidInput("UUID must be nonzero canonical lowercase".into()));
                }
                Ok(Self(id))
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                String::deserialize(d)?.parse().map_err(de::Error::custom)
            }
        }
    )+};
}
opaque_id!(
    ArtifactId,
    VariantId,
    ArtboardId,
    NodeId,
    CommentId,
    OperationId
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct RevisionId(pub u64);
impl RevisionId {
    pub fn next(self) -> DesignResult<Self> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or_else(|| DesignError::BudgetExceeded("revision counter exhausted".into()))
    }
}
impl fmt::Display for RevisionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
pub fn parse_revision_decimal(value: &str) -> DesignResult<RevisionId> {
    if value.is_empty()
        || !value.bytes().all(|b| b.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(DesignError::InvalidInput(
            "revision must be a canonical decimal string".into(),
        ));
    }
    value
        .parse()
        .map(RevisionId)
        .map_err(|_| DesignError::InvalidInput("revision exceeds u64".into()))
}
impl Serialize for RevisionId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0.to_string())
    }
}
impl<'de> Deserialize<'de> for RevisionId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        parse_revision_decimal(&String::deserialize(d)?).map_err(de::Error::custom)
    }
}
pub mod decimal_u64 {
    use super::*;
    pub fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        RevisionId(*value).serialize(serializer)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        RevisionId::deserialize(deserializer).map(|value| value.0)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SchemaVersion;
impl Serialize for SchemaVersion {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u32(1)
    }
}
impl<'de> Deserialize<'de> for SchemaVersion {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match u32::deserialize(d)? {
            1 => Ok(Self),
            _ => Err(de::Error::custom("unsupported design schema (expected 1)")),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesignKind {
    Landing,
    Product,
    Document,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesignLimits {
    pub max_files: u32,
    pub max_file_bytes: u64,
    pub max_source_bytes: u64,
    pub max_variants: u32,
    pub max_artboards: u32,
    pub max_render_bytes: u64,
    pub max_artifact_bytes: u64,
    pub max_task_bytes: u64,
}
impl Default for DesignLimits {
    fn default() -> Self {
        Self {
            max_files: 64,
            max_file_bytes: 256 * 1024,
            max_source_bytes: 2 * 1024 * 1024,
            max_variants: 3,
            max_artboards: 8,
            max_render_bytes: 50 * 1024 * 1024,
            max_artifact_bytes: 100 * 1024 * 1024,
            max_task_bytes: 500 * 1024 * 1024,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceBundle {
    #[serde(deserialize_with = "unique_files")]
    pub files: BTreeMap<String, ArtifactRef>,
    pub entry_points: Vec<String>,
}
fn unique_files<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<String, ArtifactRef>, D::Error> {
    struct Files;
    impl<'de> de::Visitor<'de> for Files {
        type Value = BTreeMap<String, ArtifactRef>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("unique source paths")
        }
        fn visit_map<M: de::MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
            let mut files = BTreeMap::new();
            while let Some((path, value)) = map.next_entry::<String, ArtifactRef>()? {
                if files.len() >= 64 || files.insert(path, value).is_some() {
                    return Err(de::Error::custom(
                        "duplicate source path or more than 64 files",
                    ));
                }
            }
            Ok(files)
        }
    }
    d.deserialize_map(Files)
}
pub fn validate_path(path: &str) -> DesignResult<()> {
    if path.is_empty() || path.len() > 240 || !path.is_ascii() {
        return Err(DesignError::InvalidInput(
            "source paths must be portable ASCII, 1..240 bytes".into(),
        ));
    }
    for part in path.split('/') {
        let stem = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        let reserved = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].contains(&stem.as_str())
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'));
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with(['.', ' '])
            || reserved
            || !part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-@".contains(&b))
        {
            return Err(DesignError::InvalidInput(format!(
                "unsafe source path: {path}"
            )));
        }
    }
    Ok(())
}
pub fn validate_hash(hash: &str) -> DesignResult<()> {
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(DesignError::InvalidInput(
            "expected lowercase SHA-256".into(),
        ));
    }
    Ok(())
}
pub fn validate_bundle(bundle: &SourceBundle, limits: &DesignLimits) -> DesignResult<()> {
    if bundle.files.is_empty() || bundle.files.len() as u64 > u64::from(limits.max_files) {
        return Err(DesignError::BudgetExceeded("source file count".into()));
    }
    let mut total = 0u64;
    let mut paths = BTreeSet::new();
    for (path, source) in &bundle.files {
        validate_path(path)?;
        validate_hash(&source.sha256)?;
        if source.relative_store_path != format!("{}.bin", source.sha256) {
            return Err(DesignError::InvalidInput(
                "invalid source artifact locator".into(),
            ));
        }
        if !paths.insert(path.to_ascii_lowercase()) {
            return Err(DesignError::InvalidInput(
                "case-fold source collision".into(),
            ));
        }
        total = total
            .checked_add(source.size)
            .ok_or_else(|| DesignError::BudgetExceeded("source size overflow".into()))?;
        if source.size > limits.max_file_bytes || total > limits.max_source_bytes {
            return Err(DesignError::BudgetExceeded("source byte limit".into()));
        }
    }
    let entries: BTreeSet<_> = bundle.entry_points.iter().collect();
    if entries.is_empty()
        || entries.len() != bundle.entry_points.len()
        || entries.iter().any(|path| !bundle.files.contains_key(*path))
    {
        return Err(DesignError::InvalidInput(
            "entry points must be unique source files".into(),
        ));
    }
    Ok(())
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artboard {
    pub id: ArtboardId,
    pub title: String,
    pub entry_point: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variant {
    pub id: VariantId,
    pub title: String,
    pub artboards: Vec<Artboard>,
}
pub fn validate_text(text: &str, maximum: u64, field: &str) -> DesignResult<()> {
    if text.trim().is_empty() || text.len() as u64 > maximum || text.contains('\0') {
        return Err(DesignError::InvalidInput(format!("invalid {field}")));
    }
    Ok(())
}
pub fn validate_variants(
    variants: &[Variant],
    bundle: &SourceBundle,
    limits: &DesignLimits,
) -> DesignResult<()> {
    if variants.is_empty() || variants.len() as u64 > u64::from(limits.max_variants) {
        return Err(DesignError::BudgetExceeded("variant count".into()));
    }
    let mut ids = BTreeSet::new();
    let mut boards = BTreeSet::new();
    for variant in variants {
        validate_text(&variant.title, 256, "variant title")?;
        if !ids.insert(variant.id) || variant.artboards.is_empty() {
            return Err(DesignError::InvalidInput(
                "duplicate variant or empty artboards".into(),
            ));
        }
        for artboard in &variant.artboards {
            validate_text(&artboard.title, 256, "artboard title")?;
            if !boards.insert(artboard.id) || !bundle.entry_points.contains(&artboard.entry_point) {
                return Err(DesignError::InvalidInput(
                    "duplicate artboard or undeclared entry point".into(),
                ));
            }
        }
    }
    if boards.len() as u64 > u64::from(limits.max_artboards) {
        return Err(DesignError::BudgetExceeded("artboard count".into()));
    }
    Ok(())
}
