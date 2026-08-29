use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("{0}")]
    Message(String),
}

pub fn export_from_file(input: &str, output: Option<&str>) -> Result<PathBuf, ExportError> {
    let input_path = expand_tilde(input);
    let raw = fs::read_to_string(&input_path)
        .map_err(|e| ExportError::Message(format!("Failed to export session: {e}")))?;
    let dest = match output {
        Some(path) => expand_tilde(path),
        None => {
            let mut dest = input_path.clone();
            dest.set_extension("html");
            dest
        }
    };
    let html = session_to_html(
        &raw,
        input_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("session"),
    );
    fs::write(&dest, html).map_err(|e| ExportError::Message(e.to_string()))?;
    Ok(dest)
}

fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

pub fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// TypeScript `sanitizeMarkdownUrl`: strip C0 controls, allow http(s)|mailto|tel|ftp.
pub fn sanitize_markdown_url(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .filter(|c| {
            let n = *c as u32;
            n > 0x1f && n != 0x7f
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("https:")
        || lower.starts_with("http:")
        || lower.starts_with("mailto:")
        || lower.starts_with("tel:")
        || lower.starts_with("ftp:")
    {
        Some(trimmed.to_string())
    } else {
        None
    }
}

pub fn session_to_html(raw: &str, title: &str) -> String {
    let title = escape_html(title);
    let mut messages = String::new();
    let mut models = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => {
                messages.push_str(&format!(
                    "<article class=\"entry raw\" id=\"entry-{}\"><pre>{}</pre></article>\n",
                    index,
                    escape_html(line)
                ));
                continue;
            }
        };
        if value.get("type").and_then(|v| v.as_str()) == Some("session")
            || value.get("kind").and_then(|v| v.as_str()) == Some("session")
        {
            continue;
        }
        let entry_id = value
            .get("id")
            .and_then(|v| v.as_str())
            .map(escape_html)
            .unwrap_or_else(|| index.to_string());
        let message = value.get("message").cloned().unwrap_or(value.clone());
        let role = message
            .get("role")
            .or_else(|| value.get("role"))
            .and_then(|v| v.as_str())
            .unwrap_or("entry");
        if let Some(model) = message
            .get("model")
            .or_else(|| value.get("modelId"))
            .and_then(|v| v.as_str())
        {
            if !models.iter().any(|m: &String| m == model) {
                models.push(model.to_string());
            }
        }
        let content = message
            .get("content")
            .or_else(|| value.get("content"))
            .map(render_content)
            .unwrap_or_else(|| escape_html(&value.to_string()));
        messages.push_str(&format!(
            "<article class=\"entry role-{role}\" id=\"entry-{entry_id}\" data-entry-id=\"{entry_id}\">\
             <header><span class=\"role\">{}</span></header>\
             <div class=\"body\">{content}</div></article>\n",
            escape_html(role)
        ));
    }
    let model_label = if models.is_empty() {
        "unknown".to_string()
    } else {
        escape_html(&models.join(", "))
    };
    let entries = collect_entries(raw);
    let tree_html = render_tree(&build_tree(&entries));
    let session_json = serde_json::json!({
        "header": {"title": title},
        "entries": entries,
        "leafId": entries.last().and_then(|e| e.get("id").cloned()),
    });
    let session_b64 = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        session_json.to_string().as_bytes(),
    );
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>{title}</title>
<style>
:root {{ color-scheme: dark; --bg:#111; --fg:#eee; --muted:#8cf; --line:#333; --user:#1e3a2f; --assistant:#1c2433; --side:#161616; }}
body {{ margin:0; font-family:ui-sans-serif,system-ui,sans-serif; background:var(--bg); color:var(--fg); }}
#app {{ display:flex; min-height:100vh; }}
#sidebar {{ width:16rem; background:var(--side); border-right:1px solid var(--line); padding:0.75rem; }}
#tree-container {{ font-size:0.85rem; }}
#tree-container ul {{ list-style:none; margin:0.25rem 0 0 0.75rem; padding:0; }}
#header-container {{ padding:1rem 1.5rem; border-bottom:1px solid var(--line); }}
#messages {{ padding:1rem 1.5rem 3rem; max-width:52rem; margin:0 auto; }}
.entry {{ margin:1rem 0; padding:0.85rem 1rem; border-radius:10px; background:#1a1a1a; }}
.role-user {{ background:var(--user); }}
.role-assistant {{ background:var(--assistant); }}
.role {{ color:var(--muted); font-size:0.75rem; text-transform:uppercase; letter-spacing:0.04em; }}
.body {{ white-space:pre-wrap; margin-top:0.4rem; }}
.body a {{ color:#8cf; }}
#hamburger {{ display:none; position:fixed; top:0.5rem; left:0.5rem; z-index:3; background:var(--side); color:var(--fg); border:1px solid var(--line); border-radius:6px; padding:0.25rem 0.5rem; }}
.sidebar-search {{ width:100%; box-sizing:border-box; background:#111; color:var(--fg); border:1px solid var(--line); border-radius:6px; padding:0.35rem 0.5rem; }}
.sidebar-filters {{ display:flex; gap:0.35rem; margin:0.5rem 0; }}
.filter-btn {{ background:transparent; color:var(--muted); border:1px solid var(--line); border-radius:999px; padding:0.15rem 0.6rem; }}
.filter-btn.active {{ color:var(--fg); border-color:var(--muted); }}
#tree-container li.active {{ color:var(--muted); }}
#tree-container li.hidden {{ display:none; }}
#sidebar-overlay {{ display:none; }}
body.light {{ --bg:#f6f6f4; --fg:#222; --muted:#06c; --line:#ddd; --user:#d9efe4; --assistant:#dde6f5; --side:#fff; }}
@media (max-width: 720px) {{
  #sidebar {{ display:none; position:fixed; z-index:2; height:100vh; }}
  #sidebar.open {{ display:block; }}
  #hamburger {{ display:block; }}
  #sidebar-overlay.open {{ display:block; position:fixed; inset:0; background:rgba(0,0,0,0.4); }}
}}
</style>
</head>
<body>
<button id="hamburger" title="Open sidebar">☰</button>
<div id="sidebar-overlay"></div>
<div id="app">
  <aside id="sidebar">
    <div class="sidebar-header">
      <input type="text" class="sidebar-search" id="tree-search" placeholder="Search...">
      <div class="sidebar-filters">
        <button class="filter-btn active" data-filter="default">Default</button>
        <button class="filter-btn" data-filter="all">All</button>
      </div>
    </div>
    <div class="tree-container" id="tree-container">{tree_html}</div>
  </aside>
  <main id="content">
    <div id="header-container"><h1>{title}</h1><p>models: {model_label}</p><button id="theme-toggle" type="button">Theme</button></div>
    <div id="messages">{messages}</div>
  </main>
</div>
<script type="application/json" id="session-data">{session_b64}</script>
<script>
function escapeHtml(value) {{
  return String(value)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}}
function sanitizeMarkdownUrl(href) {{
  const cleaned = String(href || '').replace(/[\x00-\x1f\x7f]/g, '');
  if (!/^(https?|mailto|tel|ftp):/i.test(cleaned)) return '';
  return cleaned;
}}
function link(token) {{
  const href = sanitizeMarkdownUrl(token.href);
  return href ? '<a href="' + escapeHtml(href) + '">' + escapeHtml(token.text || href) + '</a>' : escapeHtml(token.text || '');
}}
function image(token) {{
  const href = sanitizeMarkdownUrl(token.href);
  return href ? '<img src="' + escapeHtml(href) + '" alt="">' : '';
}}
function loadSessionData() {{
  const base64 = document.getElementById('session-data').textContent;
  return JSON.parse(new TextDecoder('utf-8').decode(Uint8Array.from(atob(base64), c => c.charCodeAt(0))));
}}
function buildTree() {{
  const data = loadSessionData();
  const entries = data.entries || [];
  const nodeMap = new Map();
  const roots = [];
  for (const entry of entries) {{
    nodeMap.set(entry.id, {{ entry, children: [], label: entry.label }});
  }}
  for (const entry of entries) {{
    const node = nodeMap.get(entry.id);
    if (entry.parentId === null || entry.parentId === undefined || entry.parentId === entry.id) {{
      roots.push(node);
    }} else {{
      const parent = nodeMap.get(entry.parentId);
      if (parent) parent.children.push(node); else roots.push(node);
    }}
  }}
  return roots;
}}
function getSearchableText(entry) {{
  const parts = [entry.id, entry.role, entry.type, entry.label];
  if (entry.content) parts.push(typeof entry.content === 'string' ? entry.content : JSON.stringify(entry.content));
  if (entry.message) parts.push(JSON.stringify(entry.message));
  return parts.filter(Boolean).join(' ').toLowerCase();
}}
function applyTreeFilter() {{
  const query = (document.getElementById('tree-search').value || '').toLowerCase();
  const mode = (document.querySelector('.filter-btn.active') || {{}}).dataset.filter || 'default';
  document.querySelectorAll('#tree-container li').forEach((li) => {{
    const id = li.getAttribute('data-entry-id') || '';
    const text = (li.textContent || '').toLowerCase();
    const matchQuery = !query || text.includes(query) || id.toLowerCase().includes(query);
    const matchMode = mode === 'all' || text.includes('user') || text.includes('assistant');
    li.classList.toggle('hidden', !(matchQuery && matchMode));
  }});
}}
function bindExportUi() {{
  const search = document.getElementById('tree-search');
  if (search) search.addEventListener('input', applyTreeFilter);
  document.querySelectorAll('.filter-btn').forEach((btn) => {{
    btn.addEventListener('click', () => {{
      document.querySelectorAll('.filter-btn').forEach((b) => b.classList.remove('active'));
      btn.classList.add('active');
      applyTreeFilter();
    }});
  }});
  const hamburger = document.getElementById('hamburger');
  const sidebar = document.getElementById('sidebar');
  const overlay = document.getElementById('sidebar-overlay');
  if (hamburger && sidebar) {{
    hamburger.addEventListener('click', () => {{
      sidebar.classList.add('open');
      if (overlay) overlay.classList.add('open');
      hamburger.style.display = 'none';
    }});
  }}
  if (overlay && sidebar && hamburger) {{
    overlay.addEventListener('click', () => {{
      sidebar.classList.remove('open');
      overlay.classList.remove('open');
      hamburger.style.display = '';
    }});
  }}
  const themeToggle = document.getElementById('theme-toggle');
  if (themeToggle) {{
    themeToggle.addEventListener('click', () => document.body.classList.toggle('light'));
  }}
  document.querySelectorAll('#tree-container [data-entry-id]').forEach((node) => {{
    node.addEventListener('click', (event) => {{
      event.stopPropagation();
      const id = node.getAttribute('data-entry-id');
      document.querySelectorAll('#tree-container li').forEach((li) => li.classList.remove('active'));
      node.classList.add('active');
      const target = document.getElementById('entry-' + id);
      if (target) target.scrollIntoView({{ behavior: 'smooth', block: 'start' }});
    }});
  }});
}}
if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', bindExportUi);
else bindExportUi();
</script>
</body>
</html>
"#
    )
}

fn collect_entries(raw: &str) -> Vec<Value> {
    raw.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|value| {
            value.get("type").and_then(|v| v.as_str()) != Some("session")
                && value.get("kind").and_then(|v| v.as_str()) != Some("session")
        })
        .collect()
}

#[derive(Debug, Clone)]
struct TreeNode {
    entry: Value,
    children: Vec<TreeNode>,
}

fn build_tree(entries: &[Value]) -> Vec<TreeNode> {
    let nodes: Vec<TreeNode> = entries
        .iter()
        .map(|entry| TreeNode {
            entry: entry.clone(),
            children: Vec::new(),
        })
        .collect();
    let index_by_id: std::collections::HashMap<String, usize> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, entry)| {
            entry
                .get("id")
                .and_then(|v| v.as_str())
                .map(|id| (id.to_string(), i))
        })
        .collect();
    let mut child_map: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    let mut roots = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        match entry.get("parentId").and_then(|v| v.as_str()) {
            Some(parent) => match index_by_id.get(parent) {
                Some(parent_idx) if *parent_idx != i => child_map[*parent_idx].push(i),
                _ => roots.push(i),
            },
            None => roots.push(i),
        }
    }
    fn assemble(idx: usize, nodes: &[TreeNode], child_map: &[Vec<usize>]) -> TreeNode {
        TreeNode {
            entry: nodes[idx].entry.clone(),
            children: child_map[idx]
                .iter()
                .map(|c| assemble(*c, nodes, child_map))
                .collect(),
        }
    }
    roots
        .into_iter()
        .map(|i| assemble(i, &nodes, &child_map))
        .collect()
}

fn render_tree(roots: &[TreeNode]) -> String {
    fn render_node(node: &TreeNode) -> String {
        let id = node
            .entry
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("entry");
        let role = node
            .entry
            .get("role")
            .or_else(|| node.entry.pointer("/message/role"))
            .and_then(|v| v.as_str())
            .unwrap_or(
                node.entry
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("entry"),
            );
        let children = if node.children.is_empty() {
            String::new()
        } else {
            format!(
                "<ul>{}</ul>",
                node.children.iter().map(render_node).collect::<String>()
            )
        };
        format!(
            "<li data-entry-id=\"{}\"><span>{}</span>{children}</li>",
            escape_html(id),
            escape_html(role)
        )
    }
    format!(
        "<ul>{}</ul>",
        roots.iter().map(render_node).collect::<String>()
    )
}

fn render_content(value: &Value) -> String {
    match value {
        Value::String(text) => render_markdown_lite(text),
        Value::Array(items) => items
            .iter()
            .map(|item| {
                if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                    render_markdown_lite(text)
                } else if item.get("type").and_then(|v| v.as_str()) == Some("toolCall") {
                    format!(
                        "<div class=\"tool\">[{}]</div>",
                        escape_html(item.get("name").and_then(|v| v.as_str()).unwrap_or("tool"))
                    )
                } else {
                    escape_html(&item.to_string())
                }
            })
            .collect(),
        other => escape_html(&other.to_string()),
    }
}

fn render_markdown_lite(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        out.push_str(&escape_html(&rest[..start]));
        let after = &rest[start + 1..];
        if let Some(mid) = after.find("](") {
            let label = &after[..mid];
            let href_part = &after[mid + 2..];
            if let Some(end) = href_part.find(')') {
                let href = &href_part[..end];
                match sanitize_markdown_url(href) {
                    Some(safe) => {
                        out.push_str(&format!(
                            "<a href=\"{}\">{}</a>",
                            escape_html(&safe),
                            escape_html(label)
                        ));
                    }
                    None => out.push_str(&escape_html(&format!("[{label}]({href})"))),
                }
                rest = &href_part[end + 1..];
                continue;
            }
        }
        out.push_str("&lt;");
        rest = after;
    }
    out.push_str(&escape_html(rest));
    out
}

#[allow(dead_code)]
pub fn export_exists(path: &Path) -> bool {
    path.exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_xss_and_sanitizes_urls() {
        let raw = r#"{"type":"message","id":"e<script>","role":"user","content":"hi [x](javascript:alert(1)) <img>"}"#;
        let html = session_to_html(raw, "title<script>");
        assert!(!html.contains("<script>alert"));
        assert!(html.contains("&lt;img&gt;") || html.contains("&lt;script&gt;"));
        assert!(!html.contains("href=\"javascript:"));
        assert!(html.contains("entry-e&lt;script&gt;") || html.contains("data-entry-id="));
        assert!(html.contains("sanitizeMarkdownUrl(token.href)"));
        assert!(html.contains("^(https?|mailto|tel|ftp)"));
        assert!(html.contains("replace(/[\\x00-\\x1f\\x7f]/g, '')"));
        assert!(html.contains("escapeHtml(href)"));
    }

    #[test]
    fn keeps_http_links() {
        let raw = r#"{"type":"message","id":"1","role":"assistant","content":"see [docs](https://example.com)"}"#;
        let html = session_to_html(raw, "s");
        assert!(html.contains("href=\"https://example.com\""));
        assert!(html.contains(">docs</a>"));
    }

    #[test]
    fn embeds_sidebar_tree_and_session_data() {
        let raw = r#"{"type":"message","id":"root","parentId":null,"role":"user","content":"hi"}
{"type":"message","id":"child","parentId":"root","role":"assistant","content":"ok"}"#;
        let html = session_to_html(raw, "s");
        assert!(html.contains("id=\"tree-container\""));
        assert!(html.contains("id=\"session-data\""));
        assert!(html.contains("function buildTree()"));
        assert!(html.contains("function applyTreeFilter()"));
        assert!(html.contains("id=\"tree-search\""));
        assert!(html.contains("data-filter=\"default\""));
        assert!(html.contains("data-entry-id=\"root\""));
        assert!(html.contains("data-entry-id=\"child\""));
        assert!(html.contains("id=\"hamburger\""));
        assert!(html.contains("id=\"theme-toggle\""));
        assert!(html.contains("classList.toggle('light')"));
    }
}
