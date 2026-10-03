use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Implementation {
    pub name: String,
    #[serde(default)]
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub protocol_version: String,
    #[serde(default)]
    pub capabilities: Value,
    #[serde(default)]
    pub server_info: Option<Implementation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSpec {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Providers reject a schema that is not an object, so an absent or
    /// non-object `inputSchema` becomes the empty object schema.
    #[serde(default = "default_input_schema")]
    pub input_schema: Value,
    #[serde(default)]
    pub annotations: Option<ToolAnnotations>,
}

/// `{"type":"object","properties":{}}` — the schema of a tool that takes
/// no arguments, and the fallback when a server sends none.
pub fn default_input_schema() -> Value {
    json!({ "type": "object", "properties": {} })
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ToolAnnotations {
    #[serde(default)]
    pub read_only_hint: Option<bool>,
    #[serde(default)]
    pub destructive_hint: Option<bool>,
}

impl ToolSpec {
    pub fn read_only(&self) -> bool {
        self.annotations
            .as_ref()
            .and_then(|ann| ann.read_only_hint)
            .unwrap_or(false)
    }

    /// Replace a schema no provider would accept with the empty object
    /// schema. `#[serde(default)]` only covers an absent key; an explicit
    /// `null` or a bare string lands here.
    pub fn normalize(mut self) -> Self {
        if !self.input_schema.is_object() {
            self.input_schema = default_input_schema();
        }
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resource {
    pub uri: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
}

/// One `content` block of a `tools/call` result: `text`, `image`, `audio`
/// or an embedded `resource`. Non-text blocks keep their payload so
/// [`CallToolResult::text`] can describe them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentBlock {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
    /// Base64 payload of `image` / `audio` blocks.
    #[serde(default)]
    pub data: Option<String>,
    /// `{ uri, mimeType, text | blob }` of an embedded `resource` block.
    #[serde(default)]
    pub resource: Option<Value>,
}

impl ContentBlock {
    /// The line this block contributes to the tool result. Text blocks
    /// are their text; every other block becomes one placeholder line so
    /// the model learns something arrived instead of seeing nothing.
    pub fn render(&self) -> Option<String> {
        match self.kind.as_str() {
            "text" => self.text.clone(),
            "image" | "audio" => Some(format!(
                "[{} {}, {} bytes]",
                self.kind,
                self.mime_type
                    .as_deref()
                    .unwrap_or("application/octet-stream"),
                self.data.as_deref().map(base64_decoded_len).unwrap_or(0)
            )),
            "resource" => {
                let resource = self.resource.as_ref()?;
                if let Some(text) = resource.get("text").and_then(Value::as_str) {
                    return Some(text.to_string());
                }
                let uri = resource.get("uri").and_then(Value::as_str).unwrap_or("");
                let mime = resource
                    .get("mimeType")
                    .and_then(Value::as_str)
                    .unwrap_or("application/octet-stream");
                let bytes = resource
                    .get("blob")
                    .and_then(Value::as_str)
                    .map(base64_decoded_len)
                    .unwrap_or(0);
                Some(format!("[resource {uri} {mime}, {bytes} bytes]"))
            }
            other => Some(format!("[{other}]")),
        }
    }
}

/// Byte length of a base64 payload without decoding it: six bits per
/// significant character, padding and whitespace ignored.
pub fn base64_decoded_len(encoded: &str) -> usize {
    let significant = encoded
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace() && *byte != b'=')
        .count();
    significant * 6 / 8
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallToolResult {
    #[serde(default)]
    pub content: Vec<ContentBlock>,
    #[serde(default)]
    pub is_error: Option<bool>,
}

impl CallToolResult {
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(ContentBlock::render)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// A row of `/mcp`.
#[derive(Debug, Clone)]
pub struct ServerEntry {
    pub name: String,
    pub transport: String,
    pub status: String,
    pub tools: usize,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tool_without_a_schema_gets_the_empty_object_schema() {
        let absent: ToolSpec = serde_json::from_value(json!({ "name": "t" })).unwrap();
        assert_eq!(absent.input_schema, default_input_schema());
        let null = serde_json::from_value::<ToolSpec>(json!({ "name": "t", "inputSchema": null }))
            .unwrap()
            .normalize();
        assert_eq!(null.input_schema, default_input_schema());
        let given = serde_json::from_value::<ToolSpec>(
            json!({ "name": "t", "inputSchema": { "type": "object", "properties": { "a": {} } } }),
        )
        .unwrap()
        .normalize();
        assert_eq!(given.input_schema["properties"]["a"], json!({}));
    }

    #[test]
    fn a_resource_reads_its_mime_type() {
        let resource: Resource =
            serde_json::from_value(json!({ "uri": "x://y", "mimeType": "text/plain" })).unwrap();
        assert_eq!(resource.mime_type.as_deref(), Some("text/plain"));
    }

    #[test]
    fn non_text_blocks_become_placeholder_lines() {
        let result: CallToolResult = serde_json::from_value(json!({
            "content": [
                { "type": "text", "text": "hello" },
                { "type": "image", "mimeType": "image/png", "data": "aGVsbG8=" },
                { "type": "audio", "mimeType": "audio/wav", "data": "aGk=" },
                { "type": "resource", "resource": { "uri": "f://a", "mimeType": "text/plain", "text": "inline" } },
                { "type": "resource", "resource": { "uri": "f://b", "mimeType": "application/pdf", "blob": "AAAA" } }
            ]
        }))
        .unwrap();
        assert_eq!(
            result.text(),
            "hello\n[image image/png, 5 bytes]\n[audio audio/wav, 2 bytes]\ninline\n[resource f://b application/pdf, 3 bytes]"
        );
    }

    #[test]
    fn base64_length_ignores_padding_and_whitespace() {
        assert_eq!(base64_decoded_len(""), 0);
        assert_eq!(base64_decoded_len("aGk="), 2);
        assert_eq!(base64_decoded_len("aGVs\nbG8="), 5);
        assert_eq!(base64_decoded_len("AAAA"), 3);
    }
}
