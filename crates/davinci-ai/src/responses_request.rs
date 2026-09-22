//! Immutable provider request envelope and privacy-safe wire manifest.
//!
//! The manifest fingerprints the exact JSON values DaVinci prepares for the
//! provider without retaining raw prompt text in ordinary diagnostics.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

pub const WIRE_MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireSegmentCategory {
    ModelVisibleConfig,
    ToolContract,
    TrustedInstructions,
    ConversationItem,
    RoutingPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireTrustClass {
    TrustedDeveloper,
    UserOrExternal,
    AssistantHistory,
    ToolResult,
    ProviderProtocol,
    RoutingOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireSegmentManifest {
    pub order: usize,
    pub category: WireSegmentCategory,
    pub trust: WireTrustClass,
    pub digest: String,
    pub byte_len: usize,
    pub cache_sensitive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireManifest {
    pub schema_version: u32,
    pub ordered_prefix_fingerprint: String,
    pub request_bytes_before_compression: usize,
    pub segments: Vec<WireSegmentManifest>,
}

#[derive(Debug, Clone)]
pub struct PreparedProviderRequest {
    body: Value,
    manifest: WireManifest,
}

impl PreparedProviderRequest {
    pub fn new(body: Value) -> Self {
        let request_bytes_before_compression = json_bytes(&body).len();
        let segments = build_segments(&body);
        let ordered_prefix_fingerprint = fingerprint_segments(&segments);
        Self {
            body,
            manifest: WireManifest {
                schema_version: WIRE_MANIFEST_VERSION,
                ordered_prefix_fingerprint,
                request_bytes_before_compression,
                segments,
            },
        }
    }

    pub fn body(&self) -> &Value {
        &self.body
    }

    pub fn manifest(&self) -> &WireManifest {
        &self.manifest
    }

    /// Returns the first cache-sensitive segment whose category/trust/digest
    /// differs. Routing-only metadata such as cache partition and stream mode
    /// does not masquerade as a prompt-prefix change.
    pub fn first_changed_cache_segment(&self, other: &Self) -> Option<usize> {
        let left = self
            .manifest
            .segments
            .iter()
            .filter(|segment| segment.cache_sensitive);
        let right = other
            .manifest
            .segments
            .iter()
            .filter(|segment| segment.cache_sensitive);
        let mut index = 0usize;
        let mut pairs = left.zip(right);
        while let Some((a, b)) = pairs.next() {
            if a.category != b.category
                || a.trust != b.trust
                || a.digest != b.digest
                || a.byte_len != b.byte_len
            {
                return Some(index);
            }
            index += 1;
        }
        let left_len = self
            .manifest
            .segments
            .iter()
            .filter(|segment| segment.cache_sensitive)
            .count();
        let right_len = other
            .manifest
            .segments
            .iter()
            .filter(|segment| segment.cache_sensitive)
            .count();
        (left_len != right_len).then_some(index)
    }
}

fn build_segments(body: &Value) -> Vec<WireSegmentManifest> {
    let Some(map) = body.as_object() else {
        return vec![segment(
            0,
            WireSegmentCategory::ModelVisibleConfig,
            WireTrustClass::ProviderProtocol,
            body,
            true,
        )];
    };

    let mut segments = Vec::new();
    let mut config = Map::new();
    for (key, value) in map {
        if matches!(
            key.as_str(),
            "input"
                | "messages"
                | "tools"
                | "instructions"
                | "system"
                | "prompt_cache_key"
                | "prompt_cache_retention"
                | "prompt_cache_options"
                | "stream"
                | "store"
        ) {
            continue;
        }
        config.insert(key.clone(), value.clone());
    }
    if !config.is_empty() {
        push_segment(
            &mut segments,
            WireSegmentCategory::ModelVisibleConfig,
            WireTrustClass::ProviderProtocol,
            &Value::Object(config),
            true,
        );
    }

    if let Some(tools) = map.get("tools") {
        push_segment(
            &mut segments,
            WireSegmentCategory::ToolContract,
            WireTrustClass::TrustedDeveloper,
            tools,
            true,
        );
    }

    if let Some(instructions) = map.get("instructions") {
        push_segment(
            &mut segments,
            WireSegmentCategory::TrustedInstructions,
            WireTrustClass::TrustedDeveloper,
            instructions,
            true,
        );
    }
    if let Some(system) = map.get("system") {
        push_segment(
            &mut segments,
            WireSegmentCategory::TrustedInstructions,
            WireTrustClass::TrustedDeveloper,
            system,
            true,
        );
    }

    for collection in ["input", "messages"] {
        if let Some(items) = map.get(collection).and_then(Value::as_array) {
            for item in items {
                let (category, trust) = classify_item(item);
                push_segment(&mut segments, category, trust, item, true);
            }
        }
    }

    let mut routing = Map::new();
    for key in [
        "prompt_cache_key",
        "prompt_cache_retention",
        "prompt_cache_options",
        "stream",
        "store",
    ] {
        if let Some(value) = map.get(key) {
            routing.insert(key.to_string(), value.clone());
        }
    }
    if !routing.is_empty() {
        push_segment(
            &mut segments,
            WireSegmentCategory::RoutingPolicy,
            WireTrustClass::RoutingOnly,
            &Value::Object(routing),
            false,
        );
    }

    segments
}

fn classify_item(item: &Value) -> (WireSegmentCategory, WireTrustClass) {
    let role = item.get("role").and_then(Value::as_str);
    let item_type = item.get("type").and_then(Value::as_str);
    let trust = match (role, item_type) {
        (Some("developer" | "system"), _) => WireTrustClass::TrustedDeveloper,
        (Some("assistant"), _) => WireTrustClass::AssistantHistory,
        (Some("tool"), _) | (_, Some("function_call_output" | "custom_tool_call_output")) => {
            WireTrustClass::ToolResult
        }
        (Some("user"), _) => WireTrustClass::UserOrExternal,
        (_, Some("message")) => WireTrustClass::ProviderProtocol,
        _ => WireTrustClass::ProviderProtocol,
    };
    let category = if trust == WireTrustClass::TrustedDeveloper {
        WireSegmentCategory::TrustedInstructions
    } else {
        WireSegmentCategory::ConversationItem
    };
    (category, trust)
}

fn push_segment(
    segments: &mut Vec<WireSegmentManifest>,
    category: WireSegmentCategory,
    trust: WireTrustClass,
    value: &Value,
    cache_sensitive: bool,
) {
    let order = segments.len();
    segments.push(segment(order, category, trust, value, cache_sensitive));
}

fn segment(
    order: usize,
    category: WireSegmentCategory,
    trust: WireTrustClass,
    value: &Value,
    cache_sensitive: bool,
) -> WireSegmentManifest {
    let bytes = json_bytes(value);
    WireSegmentManifest {
        order,
        category,
        trust,
        digest: digest(&bytes),
        byte_len: bytes.len(),
        cache_sensitive,
    }
}

fn fingerprint_segments(segments: &[WireSegmentManifest]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"davinci.provider-prefix-manifest.v1\0");
    for segment in segments.iter().filter(|segment| segment.cache_sensitive) {
        hasher.update((segment.category as u8).to_le_bytes());
        hasher.update((segment.trust as u8).to_le_bytes());
        hasher.update((segment.byte_len as u64).to_le_bytes());
        hasher.update(segment.digest.as_bytes());
        hasher.update([0]);
    }
    format!("{:x}", hasher.finalize())
}

fn json_bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap_or_default()
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(tail: &str, cache_key: &str) -> PreparedProviderRequest {
        PreparedProviderRequest::new(json!({
            "model": "gpt-5.6-sol",
            "store": false,
            "tools": [{"type":"function","name":"read","parameters":{"type":"object"}}],
            "input": [
                {
                    "type":"message",
                    "role":"developer",
                    "content":[{"type":"input_text","text":"stable bootstrap"}]
                },
                {
                    "type":"message",
                    "role":"user",
                    "content":[{"type":"input_text","text":tail}]
                }
            ],
            "prompt_cache_key": cache_key,
            "prompt_cache_options": {"mode":"implicit","ttl":"30m"}
        }))
    }

    #[test]
    fn changing_tail_preserves_earlier_segment_digests() {
        let a = request("question one", "partition-a");
        let b = request("question two", "partition-a");
        let change = a.first_changed_cache_segment(&b).unwrap();
        assert_eq!(change, 3);
        assert_eq!(a.manifest.segments[0], b.manifest.segments[0]);
        assert_eq!(a.manifest.segments[1], b.manifest.segments[1]);
        assert_eq!(a.manifest.segments[2], b.manifest.segments[2]);
        assert_ne!(
            a.manifest.ordered_prefix_fingerprint,
            b.manifest.ordered_prefix_fingerprint
        );
    }

    #[test]
    fn routing_key_does_not_change_prompt_prefix_fingerprint() {
        let a = request("same question", "partition-a");
        let b = request("same question", "partition-b");
        assert_eq!(a.first_changed_cache_segment(&b), None);
        assert_eq!(
            a.manifest.ordered_prefix_fingerprint,
            b.manifest.ordered_prefix_fingerprint
        );
    }

    #[test]
    fn manifest_never_contains_raw_prompt_text() {
        let prepared = request("SECRET-PROMPT-MARKER", "partition-a");
        let serialized = serde_json::to_string(prepared.manifest()).unwrap();
        assert!(!serialized.contains("SECRET-PROMPT-MARKER"));
        assert!(!serialized.contains("stable bootstrap"));
    }
}
