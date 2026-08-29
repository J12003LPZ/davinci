//! TypeScript interactive theme JSON → HTML export CSS variables.

use serde_json::{Map, Value};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

const DARK_JSON: &str =
    include_str!("../../../vendor/pi/packages/coding-agent/src/modes/interactive/theme/dark.json");
const LIGHT_JSON: &str =
    include_str!("../../../vendor/pi/packages/coding-agent/src/modes/interactive/theme/light.json");

const BASIC_ANSI: [&str; 16] = [
    "#000000", "#800000", "#008000", "#808000", "#000080", "#800080", "#008080", "#c0c0c0",
    "#808080", "#ff0000", "#00ff00", "#ffff00", "#0000ff", "#ff00ff", "#00ffff", "#ffffff",
];

pub fn builtin_theme_json(name: &str) -> &'static str {
    if is_light_theme(name) {
        LIGHT_JSON
    } else {
        DARK_JSON
    }
}

pub fn is_light_theme(name: &str) -> bool {
    name.split('/')
        .next_back()
        .unwrap_or(name)
        .eq_ignore_ascii_case("light")
}

pub fn custom_themes_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("PI_CODING_AGENT_DIR") {
        return PathBuf::from(dir).join("themes");
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".pi")
        .join("agent")
        .join("themes")
}

fn load_theme_document(theme_name: &str) -> Value {
    let name = theme_name.split('/').next_back().unwrap_or(theme_name);
    if name.eq_ignore_ascii_case("dark") || name.eq_ignore_ascii_case("light") {
        return serde_json::from_str(builtin_theme_json(name)).unwrap_or(Value::Null);
    }
    let path = custom_themes_dir().join(format!("{name}.json"));
    if let Ok(raw) = fs::read_to_string(path) {
        if let Ok(value) = serde_json::from_str::<Value>(&raw) {
            return value;
        }
    }
    serde_json::from_str(builtin_theme_json(name)).unwrap_or(Value::Null)
}

pub fn get_resolved_theme_colors(theme_name: &str) -> Map<String, Value> {
    let parsed = load_theme_document(theme_name);
    let vars = parsed
        .get("vars")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let mut colors = parsed
        .get("colors")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    apply_fallbacks(&mut colors);
    let default_text = if is_light_theme(theme_name) {
        "#000000"
    } else {
        "#e5e5e7"
    };
    let mut out = Map::new();
    for (key, value) in colors {
        match resolve_var_refs(&value, &vars, &mut HashSet::new()) {
            Ok(Value::Number(n)) => {
                if let Some(index) = n.as_u64() {
                    out.insert(key, Value::String(ansi256_to_hex(index as u16)));
                }
            }
            Ok(Value::String(s)) if s.is_empty() => {
                out.insert(key, Value::String(default_text.into()));
            }
            Ok(Value::String(s)) => {
                out.insert(key, Value::String(s));
            }
            _ => {}
        }
    }
    out
}

pub fn get_theme_export_colors(
    theme_name: &str,
) -> (Option<String>, Option<String>, Option<String>) {
    let parsed = load_theme_document(theme_name);
    let vars = parsed
        .get("vars")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let export = match parsed.get("export").and_then(|v| v.as_object()) {
        Some(map) => map,
        None => return (None, None, None),
    };
    let resolve = |key: &str| -> Option<String> {
        let value = export.get(key)?;
        match resolve_var_refs(value, &vars, &mut HashSet::new()) {
            Ok(Value::Number(n)) => n.as_u64().map(|i| ansi256_to_hex(i as u16)),
            Ok(Value::String(s)) if !s.is_empty() => Some(s),
            _ => None,
        }
    };
    (resolve("pageBg"), resolve("cardBg"), resolve("infoBg"))
}

pub fn generate_theme_vars(theme_name: &str) -> (String, String, String, String) {
    let colors = get_resolved_theme_colors(theme_name);
    let (export_page, export_card, export_info) = get_theme_export_colors(theme_name);
    let user_bg = colors
        .get("userMessageBg")
        .and_then(|v| v.as_str())
        .unwrap_or("#343541");
    let derived = derive_export_colors(user_bg);
    let page_bg = export_page.unwrap_or(derived.0);
    let card_bg = export_card.unwrap_or(derived.1);
    let info_bg = export_info.unwrap_or(derived.2);
    let mut lines = Vec::new();
    for (key, value) in &colors {
        if let Some(color) = value.as_str() {
            lines.push(format!("--{key}: {color};"));
        }
    }
    lines.push(format!("--exportPageBg: {page_bg};"));
    lines.push(format!("--exportCardBg: {card_bg};"));
    lines.push(format!("--exportInfoBg: {info_bg};"));
    (lines.join("\n      "), page_bg, card_bg, info_bg)
}

fn apply_fallbacks(colors: &mut Map<String, Value>) {
    if !colors.contains_key("thinkingMax") {
        if let Some(v) = colors.get("thinkingXhigh").cloned() {
            colors.insert("thinkingMax".into(), v);
        }
    }
    if !colors.contains_key("scrollbarThumb") {
        if let Some(v) = colors.get("selectedBg").cloned() {
            colors.insert("scrollbarThumb".into(), v);
        }
    }
    if !colors.contains_key("searchMatchBg") {
        if let Some(v) = colors.get("selectedBg").cloned() {
            colors.insert("searchMatchBg".into(), v);
        }
    }
    if !colors.contains_key("searchMatchText") {
        if let Some(v) = colors.get("text").cloned() {
            colors.insert("searchMatchText".into(), v);
        }
    }
}

fn resolve_var_refs(
    value: &Value,
    vars: &Map<String, Value>,
    visited: &mut HashSet<String>,
) -> Result<Value, String> {
    match value {
        Value::Number(_) => Ok(value.clone()),
        Value::String(s) if s.is_empty() || s.starts_with('#') || s.starts_with("rgb") => {
            Ok(value.clone())
        }
        Value::String(name) => {
            if !visited.insert(name.clone()) {
                return Err(format!("Circular variable reference detected: {name}"));
            }
            let next = vars
                .get(name)
                .ok_or_else(|| format!("Variable reference not found: {name}"))?;
            resolve_var_refs(next, vars, visited)
        }
        other => Ok(other.clone()),
    }
}

pub fn ansi256_to_hex(index: u16) -> String {
    let index = index.min(255) as usize;
    if index < 16 {
        return BASIC_ANSI[index].to_string();
    }
    if index < 232 {
        let cube = index - 16;
        let r = cube / 36;
        let g = (cube % 36) / 6;
        let b = cube % 6;
        let to_hex = |n: usize| {
            let value = if n == 0 { 0 } else { 55 + n * 40 };
            format!("{value:02x}")
        };
        return format!("#{}{}{}", to_hex(r), to_hex(g), to_hex(b));
    }
    let gray = 8 + (index - 232) * 10;
    format!("#{gray:02x}{gray:02x}{gray:02x}")
}

pub(crate) fn parse_color(color: &str) -> Option<(u8, u8, u8)> {
    if let Some(hex) = color.strip_prefix('#') {
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            return Some((r, g, b));
        }
    }
    let rgb = color.strip_prefix("rgb(")?.strip_suffix(')')?;
    let mut parts = rgb.split(',');
    let r = parts.next()?.trim().parse().ok()?;
    let g = parts.next()?.trim().parse().ok()?;
    let b = parts.next()?.trim().parse().ok()?;
    Some((r, g, b))
}

fn luminance(r: u8, g: u8, b: u8) -> f64 {
    let to_linear = |c: u8| {
        let s = f64::from(c) / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * to_linear(r) + 0.7152 * to_linear(g) + 0.0722 * to_linear(b)
}

fn adjust_brightness(color: &str, factor: f64) -> String {
    let Some((r, g, b)) = parse_color(color) else {
        return color.to_string();
    };
    let adj = |c: u8| ((f64::from(c) * factor).round() as i32).clamp(0, 255);
    format!("rgb({}, {}, {})", adj(r), adj(g), adj(b))
}

fn derive_export_colors(base: &str) -> (String, String, String) {
    let Some((r, g, b)) = parse_color(base) else {
        return (
            "rgb(24, 24, 30)".into(),
            "rgb(30, 30, 36)".into(),
            "rgb(60, 55, 40)".into(),
        );
    };
    if luminance(r, g, b) > 0.5 {
        return (
            adjust_brightness(base, 0.96),
            base.to_string(),
            format!(
                "rgb({}, {}, {})",
                (i32::from(r) + 10).min(255),
                (i32::from(g) + 5).min(255),
                (i32::from(b) - 20).max(0)
            ),
        );
    }
    (
        adjust_brightness(base, 0.7),
        adjust_brightness(base, 0.85),
        format!(
            "rgb({}, {}, {})",
            (i32::from(r) + 20).min(255),
            (i32::from(g) + 15).min(255),
            b
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_and_light_export_colors_match_typescript() {
        let (page, card, info) = get_theme_export_colors("dark");
        assert_eq!(page.as_deref(), Some("#18181e"));
        assert_eq!(card.as_deref(), Some("#1e1e24"));
        assert_eq!(info.as_deref(), Some("#3c3728"));
        let (page, card, info) = get_theme_export_colors("light");
        assert_eq!(page.as_deref(), Some("#f8f8f8"));
        assert_eq!(card.as_deref(), Some("#ffffff"));
        assert_eq!(info.as_deref(), Some("#fffae6"));
        let colors = get_resolved_theme_colors("dark");
        assert_eq!(colors["text"], "#d4d4d4");
        assert_eq!(colors["userMessageBg"], "#343541");
        assert_eq!(colors["accent"], "#8abeb7");
        let (vars, page_bg, _, _) = generate_theme_vars("light");
        assert!(vars.contains("--text:"));
        assert_eq!(page_bg, "#f8f8f8");
        assert_eq!(ansi256_to_hex(0), "#000000");
        assert_eq!(ansi256_to_hex(15), "#ffffff");
        assert_eq!(ansi256_to_hex(196), "#ff0000");
    }

    #[test]
    fn custom_theme_export_resolves_vars_like_typescript() {
        let dir = tempfile::tempdir().unwrap();
        let themes = dir.path().join("themes");
        std::fs::create_dir_all(&themes).unwrap();
        let mut dark: Value = serde_json::from_str(DARK_JSON).unwrap();
        if let Some(obj) = dark.as_object_mut() {
            obj.insert("name".into(), Value::String("custom-export-vars".into()));
            let mut vars = obj
                .get("vars")
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();
            vars.insert("pageBgVar".into(), Value::String("#112233".into()));
            vars.insert("pageBgAlias".into(), Value::String("pageBgVar".into()));
            vars.insert("infoBgVar".into(), Value::String("#445566".into()));
            vars.insert("cardBgVar".into(), Value::String("#223344".into()));
            obj.insert("vars".into(), Value::Object(vars));
            obj.insert(
                "export".into(),
                serde_json::json!({
                    "pageBg": "pageBgAlias",
                    "cardBg": "cardBgVar",
                    "infoBg": "infoBgVar"
                }),
            );
        }
        std::fs::write(
            themes.join("custom-export-vars.json"),
            serde_json::to_string(&dark).unwrap(),
        )
        .unwrap();
        let previous = std::env::var("PI_CODING_AGENT_DIR").ok();
        std::env::set_var("PI_CODING_AGENT_DIR", dir.path());
        let (page, card, info) = get_theme_export_colors("custom-export-vars");
        assert_eq!(page.as_deref(), Some("#112233"));
        assert_eq!(card.as_deref(), Some("#223344"));
        assert_eq!(info.as_deref(), Some("#445566"));
        match previous {
            Some(value) => std::env::set_var("PI_CODING_AGENT_DIR", value),
            None => std::env::remove_var("PI_CODING_AGENT_DIR"),
        }
    }
}
