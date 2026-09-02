//! `web_fetch` and `web_search`. No TypeScript counterpart; phase 3 spec,
//! "Web".
//!
//! Pages are read into text by a tolerant tag scanner rather than a DOM:
//! scripts, styles and chrome are dropped, headings keep their level, lists
//! their bullets, links their targets, `<pre>` its whitespace. Search goes to
//! Brave when a key is present and to DuckDuckGo's HTML endpoint otherwise.
//! Tests never touch the network: `PI_WEB_FETCH_FIXTURE` and
//! `PI_WEB_SEARCH_FIXTURE` name JSON files that answer instead, and
//! `PI_OFFLINE` / `PI_DISABLE_NETWORK` refuse outright.

use std::io::Read;
use std::time::Duration;

use serde_json::{json, Value};
use url::Url;

use crate::tools::{truncate_read, ToolResult};

const USER_AGENT: &str = concat!("pi-rust/", env!("CARGO_PKG_VERSION"));
const TIMEOUT: Duration = Duration::from_secs(30);
const BODY_CAP: u64 = 10 * 1024 * 1024;
const DEFAULT_RESULTS: usize = 8;
const MAX_RESULTS: usize = 20;

fn network_disabled() -> bool {
    ["PI_OFFLINE", "PI_DISABLE_NETWORK"].iter().any(|name| {
        std::env::var(name)
            .map(|value| !value.is_empty() && value != "0")
            .unwrap_or(false)
    })
}

fn fixture(name: &str) -> Option<Value> {
    let path = std::env::var(name).ok().filter(|value| !value.is_empty())?;
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// What a fetch came back with, before it is read into text.
struct Fetched {
    status: u16,
    content_type: String,
    body: String,
    final_url: String,
}

fn fetch(url: &Url) -> Result<Fetched, String> {
    if let Some(map) = fixture("PI_WEB_FETCH_FIXTURE") {
        let hit = map
            .get(url.as_str())
            .or_else(|| map.get(url.as_str().trim_end_matches('/')))
            .ok_or_else(|| format!("fixture has no entry for {url}"))?;
        return Ok(Fetched {
            status: hit.get("status").and_then(Value::as_u64).unwrap_or(200) as u16,
            content_type: hit
                .get("contentType")
                .and_then(Value::as_str)
                .unwrap_or("text/html")
                .to_string(),
            body: hit
                .get("body")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            final_url: hit
                .get("finalUrl")
                .and_then(Value::as_str)
                .unwrap_or(url.as_str())
                .to_string(),
        });
    }
    if network_disabled() {
        return Err("network disabled (PI_OFFLINE / PI_DISABLE_NETWORK)".into());
    }
    let agent = ureq::AgentBuilder::new()
        .timeout(TIMEOUT)
        .redirects(5)
        .user_agent(USER_AGENT)
        .build();
    let response = match agent
        .get(url.as_str())
        .set(
            "Accept",
            "text/html, application/json, text/plain;q=0.9, */*;q=0.5",
        )
        .call()
    {
        Ok(response) => response,
        Err(ureq::Error::Status(code, response)) => {
            let text = response.into_string().unwrap_or_default();
            return Err(format!(
                "{url} answered {code}{}",
                text.lines()
                    .next()
                    .filter(|line| !line.trim().is_empty())
                    .map(|line| format!(": {}", line.trim()))
                    .unwrap_or_default()
            ));
        }
        Err(err) => return Err(format!("could not fetch {url}: {err}")),
    };
    let status = response.status();
    let final_url = response.get_url().to_string();
    let content_type = response
        .header("content-type")
        .unwrap_or("application/octet-stream")
        .to_string();
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(BODY_CAP)
        .read_to_end(&mut bytes)
        .map_err(|err| format!("could not read {url}: {err}"))?;
    if pi_ai::trace::enabled() {
        pi_ai::trace::log(&format!(
            "web_fetch GET {url} → {status} {content_type} {}k",
            bytes.len() / 1024
        ));
    }
    Ok(Fetched {
        status,
        content_type,
        body: String::from_utf8_lossy(&bytes).into_owned(),
        final_url,
    })
}

fn parse_url(raw: &str) -> Result<Url, String> {
    let raw = raw.trim();
    let candidate = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("https://{raw}")
    };
    let url = Url::parse(&candidate).map_err(|err| format!("`{raw}` is not a URL: {err}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!(
            "`{raw}`: only http and https URLs can be fetched, not {}",
            url.scheme()
        ));
    }
    Ok(url)
}

/// `web_fetch { url, maxChars? }`.
pub fn fetch_tool(input: &Value) -> Result<ToolResult, String> {
    let raw = input
        .get("url")
        .and_then(Value::as_str)
        .ok_or("Missing url")?;
    let url = parse_url(raw)?;
    let fetched = fetch(&url)?;
    let kind = fetched
        .content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let (title, text) =
        if kind.contains("html") || kind.contains("xml") && fetched.body.contains("<html") {
            let (title, text) = html_to_text(&fetched.body, Some(&url));
            (title, text)
        } else if kind.starts_with("text/")
            || kind.contains("json")
            || kind.contains("xml")
            || kind.contains("javascript")
            || kind.is_empty()
        {
            (None, fetched.body.clone())
        } else {
            return Err(format!(
                "{url} is {kind}, which web_fetch cannot read as text"
            ));
        };
    let mut body = match title {
        Some(title) if !text.starts_with(&title) => format!("{title}\n\n{text}"),
        _ => text,
    };
    if let Some(max) = input
        .get("maxChars")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .filter(|n| *n > 0)
    {
        if body.chars().count() > max {
            body = body.chars().take(max).collect::<String>()
                + &format!("\n\n[truncated to {max} characters]");
        }
    }
    let (content, truncation) = truncate_read(&body, 1, None);
    let mut content = content;
    if truncation
        .get("truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        content.push_str(&format!(
            "\n\n[Showing {} of {} lines. Use maxChars or ask for a section.]",
            truncation["outputLines"], truncation["totalLines"]
        ));
    }
    Ok(ToolResult {
        content,
        is_error: false,
        details: Some(json!({
            "url": url.as_str(),
            "finalUrl": fetched.final_url,
            "status": fetched.status,
            "contentType": fetched.content_type,
            "truncation": truncation,
        })),
    })
}

/// One hit of a search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

fn brave_key() -> Option<String> {
    std::env::var("BRAVE_API_KEY")
        .ok()
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
}

fn search(query: &str, limit: usize) -> Result<(String, Vec<SearchResult>), String> {
    if let Some(map) = fixture("PI_WEB_SEARCH_FIXTURE") {
        let hits = map
            .get(query)
            .and_then(Value::as_array)
            .ok_or_else(|| format!("fixture has no entry for `{query}`"))?;
        let results = hits
            .iter()
            .map(|hit| SearchResult {
                title: hit
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                url: hit
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                snippet: hit
                    .get("snippet")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            })
            .take(limit)
            .collect();
        return Ok(("fixture".into(), results));
    }
    if network_disabled() {
        return Err("network disabled (PI_OFFLINE / PI_DISABLE_NETWORK)".into());
    }
    let agent = ureq::AgentBuilder::new()
        .timeout(TIMEOUT)
        .redirects(5)
        .user_agent(USER_AGENT)
        .build();
    if let Some(key) = brave_key() {
        let response = agent
            .get("https://api.search.brave.com/res/v1/web/search")
            .query("q", query)
            .query("count", &limit.to_string())
            .set("Accept", "application/json")
            .set("X-Subscription-Token", &key)
            .call()
            .map_err(|err| format!("Brave search failed: {err}"))?;
        let body: Value = response
            .into_json()
            .map_err(|err| format!("Brave search answered badly: {err}"))?;
        let results = body
            .get("web")
            .and_then(|web| web.get("results"))
            .and_then(Value::as_array)
            .map(|hits| {
                hits.iter()
                    .map(|hit| SearchResult {
                        title: hit
                            .get("title")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        url: hit
                            .get("url")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        snippet: strip_tags(
                            hit.get("description").and_then(Value::as_str).unwrap_or(""),
                        ),
                    })
                    .take(limit)
                    .collect()
            })
            .unwrap_or_default();
        return Ok(("brave".into(), results));
    }
    let response = agent
        .get("https://html.duckduckgo.com/html/")
        .query("q", query)
        .call()
        .map_err(|err| format!("DuckDuckGo search failed: {err}"))?;
    let html = response
        .into_string()
        .map_err(|err| format!("DuckDuckGo answered badly: {err}"))?;
    if pi_ai::trace::enabled() {
        pi_ai::trace::log(&format!(
            "web_search duckduckgo `{query}` {}k",
            html.len() / 1024
        ));
    }
    let mut results = duckduckgo_results(&html);
    results.truncate(limit);
    Ok(("duckduckgo".into(), results))
}

/// `web_search { query, limit? }`.
pub fn search_tool(input: &Value) -> Result<ToolResult, String> {
    let query = input
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .ok_or("Missing query")?;
    let limit = input
        .get("limit")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_RESULTS)
        .min(MAX_RESULTS);
    let (provider, results) = search(query, limit)?;
    let content = if results.is_empty() {
        format!("No results for `{query}`.")
    } else {
        results
            .iter()
            .enumerate()
            .map(|(index, hit)| {
                let mut row = format!("{}. {}\n   {}", index + 1, hit.title, hit.url);
                if !hit.snippet.is_empty() {
                    row.push_str("\n   ");
                    row.push_str(&hit.snippet);
                }
                row
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    Ok(ToolResult {
        content,
        is_error: false,
        details: Some(json!({
            "provider": provider,
            "query": query,
            "results": results.iter().map(|hit| json!({
                "title": hit.title, "url": hit.url, "snippet": hit.snippet
            })).collect::<Vec<_>>(),
        })),
    })
}

/// The result anchors of DuckDuckGo's HTML endpoint: `class="result__a"`
/// for the title and target (a redirect whose `uddg` query carries the real
/// URL), `class="result__snippet"` for the description.
pub fn duckduckgo_results(html: &str) -> Vec<SearchResult> {
    let mut results = Vec::new();
    let mut cursor = 0usize;
    while let Some(offset) = html[cursor..].find("result__a") {
        let at = cursor + offset;
        let Some(tag_start) = html[..at].rfind("<a") else {
            cursor = at + 9;
            continue;
        };
        let Some(tag_end) = html[at..].find('>') else {
            break;
        };
        let tag = &html[tag_start..at + tag_end + 1];
        let href = attribute(tag, "href").unwrap_or_default();
        let body_start = at + tag_end + 1;
        let Some(close) = html[body_start..].find("</a>") else {
            break;
        };
        let title = strip_tags(&html[body_start..body_start + close]);
        cursor = body_start + close;
        let snippet = html[cursor..]
            .find("result__snippet")
            .and_then(|offset| {
                let start = cursor + offset;
                // Only the snippet of this result, not the next one's.
                if html[cursor..start].contains("result__a") {
                    return None;
                }
                let end = html[start..].find('>')? + start + 1;
                let close = html[end..]
                    .find("</a>")
                    .or_else(|| html[end..].find("</div>"))?
                    + end;
                Some(strip_tags(&html[end..close]))
            })
            .unwrap_or_default();
        let url = resolve_duckduckgo_href(&href);
        if !url.is_empty() && !title.is_empty() {
            results.push(SearchResult {
                title,
                url,
                snippet,
            });
        }
    }
    results
}

fn resolve_duckduckgo_href(href: &str) -> String {
    let href = decode_entities(href);
    let absolute = if href.starts_with("//") {
        format!("https:{href}")
    } else {
        href.clone()
    };
    if let Ok(parsed) = Url::parse(&absolute) {
        if let Some((_, target)) = parsed.query_pairs().find(|(key, _)| key == "uddg") {
            return target.into_owned();
        }
    }
    absolute
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut search = 0usize;
    while let Some(offset) = lower[search..].find(name) {
        let at = search + offset;
        let before_ok = at == 0
            || lower[..at]
                .chars()
                .next_back()
                .is_some_and(|ch| ch.is_whitespace());
        let rest = &tag[at + name.len()..];
        let rest_trim = rest.trim_start();
        if before_ok && rest_trim.starts_with('=') {
            let value = rest_trim[1..].trim_start();
            let quote = value.chars().next()?;
            return if quote == '"' || quote == '\'' {
                value[1..]
                    .find(quote)
                    .map(|end| value[1..1 + end].to_string())
            } else {
                Some(
                    value
                        .split(|ch: char| ch.is_whitespace() || ch == '>')
                        .next()
                        .unwrap_or("")
                        .to_string(),
                )
            };
        }
        search = at + name.len();
    }
    None
}

/// Tags gone, entities decoded, whitespace collapsed — for a snippet.
pub fn strip_tags(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    decode_entities(&out)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// `&amp;`, `&#39;`, `&#x27;` and the named entities a page is likely to
/// carry.
pub fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let Some(end) = tail.find(';').filter(|end| *end <= 10) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let entity = &tail[1..end];
        let decoded = if let Some(number) = entity.strip_prefix('#') {
            let code = if let Some(hex) = number.strip_prefix(['x', 'X']) {
                u32::from_str_radix(hex, 16).ok()
            } else {
                number.parse::<u32>().ok()
            };
            code.and_then(char::from_u32).map(|ch| ch.to_string())
        } else {
            named_entity(entity).map(str::to_string)
        };
        match decoded {
            Some(value) => {
                out.push_str(&value);
                rest = &tail[end + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn named_entity(name: &str) -> Option<&'static str> {
    Some(match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" => " ",
        "copy" => "©",
        "reg" => "®",
        "trade" => "™",
        "mdash" => "—",
        "ndash" => "–",
        "hellip" => "…",
        "laquo" => "«",
        "raquo" => "»",
        "ldquo" => "“",
        "rdquo" => "”",
        "lsquo" => "‘",
        "rsquo" => "’",
        "bull" => "•",
        "middot" => "·",
        "times" => "×",
        "deg" => "°",
        "euro" => "€",
        "pound" => "£",
        "yen" => "¥",
        "para" => "¶",
        "sect" => "§",
        "larr" => "←",
        "rarr" => "→",
        "uarr" => "↑",
        "darr" => "↓",
        "hearts" => "♥",
        "check" => "✓",
        _ => return None,
    })
}

/// Elements whose content is never text worth reading.
const SKIPPED: &[&str] = &[
    "script", "style", "noscript", "svg", "template", "iframe", "canvas", "nav", "header",
    "footer", "head",
];

/// Elements that start a new line before and after.
const BLOCKS: &[&str] = &[
    "p",
    "div",
    "section",
    "article",
    "main",
    "aside",
    "li",
    "ul",
    "ol",
    "tr",
    "table",
    "thead",
    "tbody",
    "blockquote",
    "pre",
    "hr",
    "dt",
    "dd",
    "dl",
    "form",
    "figure",
    "figcaption",
    "details",
    "summary",
    "address",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "body",
    "html",
    "fieldset",
    "legend",
    "option",
    "select",
    "textarea",
    "label",
    "button",
];

/// A page as text: the `<title>` and the readable body.
pub fn html_to_text(html: &str, base: Option<&Url>) -> (Option<String>, String) {
    let mut reader = TextReader {
        base,
        ..TextReader::default()
    };
    reader.run(html);
    reader.finish()
}

#[derive(Default)]
struct TextReader<'a> {
    base: Option<&'a Url>,
    title: Option<String>,
    lines: Vec<String>,
    current: String,
    /// Inline text since the innermost `<a>` opened, and its target.
    link: Option<(String, usize)>,
    pre_depth: usize,
    list_depth: usize,
    ordered: Vec<Option<usize>>,
    heading: Option<usize>,
    quote_depth: usize,
    in_code: bool,
    /// Whether a blank row should precede the next line.
    want_gap: bool,
}

impl TextReader<'_> {
    fn run(&mut self, html: &str) {
        let mut rest = html;
        while !rest.is_empty() {
            let Some(lt) = rest.find('<') else {
                self.text(rest);
                break;
            };
            self.text(&rest[..lt]);
            rest = &rest[lt..];
            if rest.starts_with("<!--") {
                rest = rest.find("-->").map(|end| &rest[end + 3..]).unwrap_or("");
                continue;
            }
            if rest.starts_with("<!") || rest.starts_with("<?") {
                rest = rest.find('>').map(|end| &rest[end + 1..]).unwrap_or("");
                continue;
            }
            let Some(end) = tag_end(rest) else {
                break;
            };
            let tag = &rest[1..end];
            rest = &rest[end + 1..];
            let closing = tag.starts_with('/');
            let name_part = tag.trim_start_matches('/');
            let name: String = name_part
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase();
            if name.is_empty() {
                continue;
            }
            if name == "title" && !closing {
                if let Some(close) = find_close(rest, "title") {
                    let title = strip_tags(&rest[..close]);
                    if !title.is_empty() && self.title.is_none() {
                        self.title = Some(title);
                    }
                    rest = skip_close(rest, close, "title");
                }
                continue;
            }
            if !closing && SKIPPED.contains(&name.as_str()) {
                if name == "head" {
                    // The head is skipped except for its title.
                    if let Some(close) = find_close(rest, "head") {
                        let head = &rest[..close];
                        if self.title.is_none() {
                            if let Some(start) = head.to_ascii_lowercase().find("<title") {
                                let after = &head[start..];
                                if let Some(gt) = after.find('>') {
                                    let body = &after[gt + 1..];
                                    let close_title = body.to_ascii_lowercase().find("</title");
                                    let title =
                                        strip_tags(&body[..close_title.unwrap_or(body.len())]);
                                    if !title.is_empty() {
                                        self.title = Some(title);
                                    }
                                }
                            }
                        }
                        rest = skip_close(rest, close, "head");
                    }
                    continue;
                }
                if let Some(close) = find_close(rest, &name) {
                    rest = skip_close(rest, close, &name);
                } else {
                    rest = "";
                }
                continue;
            }
            self.tag(&name, name_part, closing);
        }
    }

    fn tag(&mut self, name: &str, raw: &str, closing: bool) {
        match name {
            "br" => self.flush(false),
            "hr" => {
                self.flush(true);
                self.lines.push("---".into());
                self.want_gap = true;
            }
            "pre" => {
                self.flush(true);
                if closing {
                    self.pre_depth = self.pre_depth.saturating_sub(1);
                    self.lines.push("```".into());
                    self.want_gap = true;
                } else {
                    self.lines.push("```".into());
                    self.pre_depth += 1;
                }
            }
            "code" | "kbd" | "samp" | "tt" if self.pre_depth == 0 => {
                if closing && self.in_code {
                    self.current.push('`');
                    self.in_code = false;
                } else if !closing && !self.in_code {
                    self.current.push('`');
                    self.in_code = true;
                }
            }
            "a" => {
                if closing {
                    if let Some((text, start)) = self.link.take() {
                        let inner = self.current[start..].trim().to_string();
                        if let Some(href) = self.resolve(&text) {
                            let bare = inner.trim_matches('`');
                            if !inner.is_empty()
                                && bare != href
                                && !href.starts_with('#')
                                && !href.starts_with("javascript:")
                            {
                                self.current.push_str(&format!(" ({href})"));
                            } else if inner.is_empty() && !href.starts_with('#') {
                                self.current.push_str(&href);
                            }
                        }
                    }
                } else if let Some(href) = attribute(raw, "href") {
                    self.link = Some((href, self.current.len()));
                }
            }
            "img" => {
                if let Some(alt) = attribute(raw, "alt").filter(|alt| !alt.trim().is_empty()) {
                    self.current.push_str(&format!("[image: {}]", alt.trim()));
                }
            }
            "ul" | "ol" => {
                self.flush(true);
                if closing {
                    self.list_depth = self.list_depth.saturating_sub(1);
                    self.ordered.pop();
                    if self.list_depth == 0 {
                        self.want_gap = true;
                    }
                } else {
                    self.list_depth += 1;
                    self.ordered.push(if name == "ol" { Some(0) } else { None });
                }
            }
            "li" => {
                self.flush(false);
                if !closing {
                    let indent = "  ".repeat(self.list_depth.saturating_sub(1));
                    let marker = match self.ordered.last_mut() {
                        Some(Some(count)) => {
                            *count += 1;
                            format!("{count}. ")
                        }
                        _ => "- ".into(),
                    };
                    self.current.push_str(&format!("{indent}{marker}"));
                }
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.flush(true);
                if closing {
                    self.heading = None;
                    self.want_gap = true;
                } else {
                    let level = name[1..].parse::<usize>().unwrap_or(1);
                    self.heading = Some(level);
                    self.current.push_str(&"#".repeat(level));
                    self.current.push(' ');
                }
            }
            "blockquote" => {
                self.flush(true);
                if closing {
                    self.quote_depth = self.quote_depth.saturating_sub(1);
                    self.want_gap = true;
                } else {
                    self.quote_depth += 1;
                }
            }
            "td" | "th" => {
                if !closing && !self.current.trim().is_empty() {
                    self.current.push_str(" | ");
                }
            }
            "p" => self.flush(true),
            _ if BLOCKS.contains(&name) => self.flush(false),
            _ => {}
        }
    }

    fn resolve(&self, href: &str) -> Option<String> {
        let href = decode_entities(href.trim());
        if href.is_empty() {
            return None;
        }
        match self.base {
            Some(base) => base
                .join(&href)
                .map(|url| url.to_string())
                .ok()
                .or(Some(href)),
            None => Some(href),
        }
    }

    fn text(&mut self, raw: &str) {
        if raw.is_empty() {
            return;
        }
        let decoded = decode_entities(raw);
        if self.pre_depth > 0 {
            for (index, line) in decoded.split('\n').enumerate() {
                if index > 0 {
                    self.lines.push(std::mem::take(&mut self.current));
                }
                self.current.push_str(line.trim_end_matches('\r'));
            }
            return;
        }
        let leading = decoded.chars().next().is_some_and(char::is_whitespace);
        let trailing = decoded.chars().last().is_some_and(char::is_whitespace);
        let words: Vec<&str> = decoded.split_whitespace().collect();
        if words.is_empty() {
            if leading && !self.current.is_empty() && !self.current.ends_with(' ') {
                self.current.push(' ');
            }
            return;
        }
        if leading && !self.current.is_empty() && !self.current.ends_with(' ') {
            self.current.push(' ');
        }
        self.current.push_str(&words.join(" "));
        if trailing {
            self.current.push(' ');
        }
    }

    /// Close the line being built; `gap` asks for a blank row after it.
    fn flush(&mut self, gap: bool) {
        let line = std::mem::take(&mut self.current);
        let trimmed = if self.pre_depth > 0 {
            line
        } else {
            line.trim().to_string()
        };
        if trimmed.is_empty() {
            if gap && !self.lines.is_empty() {
                self.want_gap = true;
            }
            return;
        }
        if self.want_gap && self.lines.last().is_some_and(|last| !last.is_empty()) {
            self.lines.push(String::new());
        }
        self.want_gap = gap;
        let prefixed = if self.quote_depth > 0 && self.pre_depth == 0 {
            format!("{}{trimmed}", "> ".repeat(self.quote_depth))
        } else {
            trimmed
        };
        self.lines.push(prefixed);
    }

    fn finish(mut self) -> (Option<String>, String) {
        self.flush(false);
        while self.lines.last().is_some_and(|last| last.trim().is_empty()) {
            self.lines.pop();
        }
        (self.title, self.lines.join("\n"))
    }
}

/// The index of the `>` that closes a tag opened at `rest[0]`, honouring
/// quoted attribute values.
fn tag_end(rest: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (index, ch) in rest.char_indices().skip(1) {
        match quote {
            Some(open) if ch == open => quote = None,
            Some(_) => {}
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if ch == '>' => return Some(index),
            None => {}
        }
    }
    None
}

fn find_close(rest: &str, name: &str) -> Option<usize> {
    let lower = rest.to_ascii_lowercase();
    let needle = format!("</{name}");
    lower.find(&needle)
}

fn skip_close<'a>(rest: &'a str, close: usize, _name: &str) -> &'a str {
    let after = &rest[close..];
    after.find('>').map(|gt| &after[gt + 1..]).unwrap_or("")
}

pub fn fetch_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "url": {"type": "string", "description": "The http(s) URL to read"},
            "maxChars": {"type": "integer", "description": "Cut the text after this many characters (optional)"}
        },
        "required": ["url"]
    })
}

pub fn search_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "query": {"type": "string", "description": "What to search for"},
            "limit": {"type": "integer", "description": "How many results (default 8, max 20)"}
        },
        "required": ["query"]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    static ENV: Mutex<()> = Mutex::new(());

    fn env_lock() -> MutexGuard<'static, ()> {
        ENV.lock().unwrap_or_else(|err| err.into_inner())
    }

    const PAGE: &str = r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>Myers &amp; friends</title>
<style>body{color:red}</style><script>alert(1)</script></head>
<body>
<nav><a href="/">Home</a></nav>
<h1>A diff   algorithm</h1>
<p>It runs in <code>O(ND)</code> time &mdash; see <a href="/paper">the paper</a>
and <a href="https://example.com/x">https://example.com/x</a>.</p>
<ul><li>first</li><li>second <b>bold</b></li></ul>
<ol><li>one</li><li>two</li></ol>
<pre>let x = 1;
  let y = 2;</pre>
<blockquote>quoted words</blockquote>
<table><tr><th>a</th><th>b</th></tr><tr><td>1</td><td>2</td></tr></table>
<img alt="a chart" src="c.png">
<footer>copyright</footer>
</body></html>"#;

    #[test]
    fn a_page_reads_as_text_with_its_structure_kept() {
        let base = Url::parse("https://example.com/docs/").unwrap();
        let (title, text) = html_to_text(PAGE, Some(&base));
        assert_eq!(title.as_deref(), Some("Myers & friends"));
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "# A diff algorithm");
        assert_eq!(lines[1], "");
        assert_eq!(
            lines[2],
            "It runs in `O(ND)` time — see the paper (https://example.com/paper) and https://example.com/x."
        );
        assert!(lines.contains(&"- first"), "{text}");
        assert!(lines.contains(&"- second bold"), "{text}");
        assert!(lines.contains(&"1. one"), "{text}");
        assert!(lines.contains(&"2. two"), "{text}");
        assert!(lines.contains(&"```"), "{text}");
        assert!(lines.contains(&"  let y = 2;"), "{text}");
        assert!(lines.contains(&"> quoted words"), "{text}");
        assert!(lines.contains(&"a | b"), "{text}");
        assert!(lines.contains(&"1 | 2"), "{text}");
        assert!(lines.contains(&"[image: a chart]"), "{text}");
        assert!(!text.contains("alert"), "{text}");
        assert!(!text.contains("color:red"), "{text}");
        assert!(!text.contains("Home"), "{text}");
        assert!(!text.contains("copyright"), "{text}");
    }

    #[test]
    fn entities_and_tags_decode() {
        assert_eq!(
            decode_entities("a &amp; b &#39;c&#x27; &nbsp;&unknown; &"),
            "a & b 'c'  &unknown; &"
        );
        assert_eq!(strip_tags("<b>bold</b>  and <i>x</i>"), "bold and x");
        assert_eq!(
            attribute(r#"a class="x" href='//d.com/l/?uddg=1'"#, "href").as_deref(),
            Some("//d.com/l/?uddg=1")
        );
        assert_eq!(
            attribute("a data-href=\"no\" HREF=yes>", "href").as_deref(),
            Some("yes")
        );
    }

    const DDG: &str = r#"<div class="result results_links results_links_deep web-result ">
<div class="links_main links_deep result__body">
<h2 class="result__title"><a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fmyers&amp;rut=abc">Myers <b>diff</b> algorithm</a></h2>
<a class="result__snippet" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fmyers&amp;rut=abc">An O(ND) <b>difference</b> algorithm &amp; its variations.</a>
<div class="result__extras"><a class="result__url" href="...">example.com/myers</a></div>
</div></div>
<div class="result"><h2 class="result__title"><a class="result__a" href="https://direct.example.org/">Direct</a></h2></div>"#;

    #[test]
    fn duckduckgo_results_are_read_from_their_anchors() {
        let hits = duckduckgo_results(DDG);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].title, "Myers diff algorithm");
        assert_eq!(hits[0].url, "https://example.com/myers");
        assert_eq!(
            hits[0].snippet,
            "An O(ND) difference algorithm & its variations."
        );
        assert_eq!(hits[1].title, "Direct");
        assert_eq!(hits[1].url, "https://direct.example.org/");
        assert_eq!(hits[1].snippet, "");
    }

    #[test]
    fn fixtures_answer_fetch_and_search_and_offline_refuses() {
        let _guard = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let fetch_path = dir.path().join("fetch.json");
        std::fs::write(
            &fetch_path,
            serde_json::to_string(&json!({
                "https://example.com/doc": {"status": 200, "contentType": "text/html; charset=utf-8", "body": PAGE},
                "https://example.com/raw.txt": {"contentType": "text/plain", "body": "plain body"},
                "https://example.com/bin": {"contentType": "application/pdf", "body": "%PDF"}
            }))
            .unwrap(),
        )
        .unwrap();
        let search_path = dir.path().join("search.json");
        std::fs::write(
            &search_path,
            serde_json::to_string(&json!({
                "rust myers diff": [
                    {"title": "Myers", "url": "https://example.com/myers", "snippet": "An algorithm"},
                    {"title": "Other", "url": "https://example.com/other"}
                ]
            }))
            .unwrap(),
        )
        .unwrap();
        std::env::set_var("PI_WEB_FETCH_FIXTURE", &fetch_path);
        std::env::set_var("PI_WEB_SEARCH_FIXTURE", &search_path);

        let page = fetch_tool(&json!({"url": "https://example.com/doc"})).unwrap();
        assert!(
            page.content
                .starts_with("Myers & friends\n\n# A diff algorithm"),
            "{}",
            page.content
        );
        assert_eq!(page.details.as_ref().unwrap()["status"], 200);
        let raw = fetch_tool(&json!({"url": "example.com/raw.txt"})).unwrap();
        assert_eq!(raw.content, "plain body");
        let cut =
            fetch_tool(&json!({"url": "https://example.com/raw.txt", "maxChars": 5})).unwrap();
        assert!(
            cut.content
                .starts_with("plain\n\n[truncated to 5 characters]"),
            "{}",
            cut.content
        );
        let binary = fetch_tool(&json!({"url": "https://example.com/bin"})).unwrap_err();
        assert!(binary.contains("application/pdf"), "{binary}");
        let scheme = fetch_tool(&json!({"url": "file:///etc/passwd"})).unwrap_err();
        assert!(scheme.contains("only http and https"), "{scheme}");

        let hits = search_tool(&json!({"query": "rust myers diff", "limit": 1})).unwrap();
        assert_eq!(
            hits.content,
            "1. Myers\n   https://example.com/myers\n   An algorithm"
        );
        assert_eq!(hits.details.as_ref().unwrap()["provider"], "fixture");
        let all = search_tool(&json!({"query": "rust myers diff"})).unwrap();
        assert!(
            all.content
                .contains("2. Other\n   https://example.com/other"),
            "{}",
            all.content
        );
        assert!(search_tool(&json!({"query": "  "})).is_err());

        std::env::remove_var("PI_WEB_FETCH_FIXTURE");
        std::env::remove_var("PI_WEB_SEARCH_FIXTURE");
        std::env::set_var("PI_OFFLINE", "1");
        let refused = fetch_tool(&json!({"url": "https://example.com/doc"})).unwrap_err();
        assert!(refused.starts_with("network disabled"), "{refused}");
        let refused = search_tool(&json!({"query": "x"})).unwrap_err();
        assert!(refused.starts_with("network disabled"), "{refused}");
        std::env::remove_var("PI_OFFLINE");
    }
}
