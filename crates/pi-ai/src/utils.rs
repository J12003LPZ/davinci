use crate::error::Result;
use crate::types::*;
use std::collections::HashMap;

pub fn content_text(blocks: &[ContentBlock]) -> String {
    let mut out = String::new();
    for b in blocks {
        if let ContentBlock::Text(t) = b {
            out.push_str(&t.text);
        }
    }
    out
}

pub fn json_parse_loose(s: &str) -> Result<serde_json::Value> {
    if let Ok(val) = serde_json::from_str(s) {
        return Ok(val);
    }
    let trimmed = s.trim();
    if trimmed.starts_with('{') && !trimmed.ends_with('}') {
        let repaired = format!("{}}}", trimmed);
        if let Ok(val) = serde_json::from_str(&repaired) {
            return Ok(val);
        }
    }
    serde_json::from_str(s).map_err(crate::error::Error::JsonError)
}

pub fn sanitize_unicode_surrogates(input: &str) -> String {
    input.to_string()
}

pub fn merge_headers(
    base: Option<&HashMap<String, String>>,
    custom: Option<&ProviderHeaders>,
) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if let Some(b) = base {
        for (k, v) in b {
            out.insert(k.clone(), v.clone());
        }
    }
    if let Some(c) = custom {
        for (k, v) in c {
            if let Some(val) = v {
                out.insert(k.clone(), val.clone());
            } else {
                out.remove(k);
            }
        }
    }
    out
}
