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
//!
//! The model chooses the URL, so `web_fetch` guards against server-side
//! request forgery: local hostnames and loopback, private, link-local,
//! unspecified, multicast and broadcast addresses (literal, DNS-resolved, or
//! IPv4-mapped IPv6) are refused before any connection, and again on every
//! redirect hop, which the tool follows by hand. Set
//! `PI_WEB_FETCH_ALLOW_PRIVATE=1` to fetch from a development server on such
//! an address.

use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::time::Duration;

use serde_json::{json, Value};
use url::{Host, Url};

use crate::tools::{truncate_read, ToolResult};

const USER_AGENT: &str = concat!("pi-rust/", env!("CARGO_PKG_VERSION"));
const TIMEOUT: Duration = Duration::from_secs(30);
const BODY_CAP: u64 = 10 * 1024 * 1024;
const BODY_CAP_LABEL: &str = "10 MB";
const MAX_REDIRECTS: usize = 5;
const DEFAULT_RESULTS: usize = 8;
const MAX_RESULTS: usize = 20;

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|value| !value.is_empty() && value != "0")
        .unwrap_or(false)
}

/// `PI_WEB_FETCH_ALLOW_PRIVATE=1` lifts the address guard so a developer can
/// point `web_fetch` at `http://localhost:3000` or a machine on the LAN. The
/// scheme check and the redirect cap stay. Off by default; never set it in a
/// shared or CI environment.
fn allow_private() -> bool {
    env_flag("PI_WEB_FETCH_ALLOW_PRIVATE")
}

fn network_disabled() -> bool {
    ["PI_OFFLINE", "PI_DISABLE_NETWORK"]
        .iter()
        .any(|name| env_flag(name))
}

/// Why an address may not be fetched, or `None` when it may.
fn ip_refusal(ip: IpAddr) -> Option<&'static str> {
    match ip {
        IpAddr::V4(v4) => ipv4_refusal(v4),
        IpAddr::V6(v6) => ipv6_refusal(v6),
    }
}

fn ipv4_refusal(ip: Ipv4Addr) -> Option<&'static str> {
    if ip.is_unspecified() {
        Some("unspecified address")
    } else if ip.is_loopback() {
        Some("loopback address")
    } else if ip.is_link_local() {
        // 169.254/16, which includes the cloud metadata endpoint.
        Some("link-local address")
    } else if ip.is_private() {
        Some("private address")
    } else if ip.is_broadcast() {
        Some("broadcast address")
    } else if ip.is_multicast() {
        Some("multicast address")
    } else {
        None
    }
}

fn ipv6_refusal(ip: Ipv6Addr) -> Option<&'static str> {
    if ip.is_unspecified() {
        return Some("unspecified address");
    }
    if ip.is_loopback() {
        return Some("loopback address");
    }
    if ip.is_multicast() {
        return Some("multicast address");
    }
    // `::ffff:a.b.c.d` (mapped) and `::a.b.c.d` (compatible) forms of an
    // IPv4 address are judged as that address. `to_ipv4` would also turn
    // `::1` into `0.0.0.1`, which is why loopback is checked first.
    if let Some(v4) = ip.to_ipv4() {
        return ipv4_refusal(v4);
    }
    let first = ip.segments()[0];
    if first & 0xfe00 == 0xfc00 {
        // fc00::/7, unique local.
        return Some("private address");
    }
    if first & 0xffc0 == 0xfe80 {
        // fe80::/10.
        return Some("link-local address");
    }
    None
}

/// A hostname that names this machine or the local network, whatever DNS
/// says about it.
fn local_name_refusal(name: &str) -> Option<&'static str> {
    let name = name.trim_end_matches('.').to_ascii_lowercase();
    if name == "localhost" || name.ends_with(".localhost") {
        Some("local hostname")
    } else if name.ends_with(".local") {
        Some("mDNS hostname")
    } else {
        None
    }
}

/// Resolve `host:port` and refuse when any of its addresses is one the
/// guard rejects; the caller decides whether to consult DNS at all.
fn resolved_refusal(host: &str, port: u16) -> Result<Vec<SocketAddr>, String> {
    let addrs: Vec<SocketAddr> = (host, port)
        .to_socket_addrs()
        .map_err(|err| format!("could not resolve {host}: {err}"))?
        .collect();
    if addrs.is_empty() {
        return Err(format!("could not resolve {host}: no addresses"));
    }
    for addr in &addrs {
        if let Some(why) = ip_refusal(addr.ip()) {
            return Err(format!("{host} resolves to {} ({why})", addr.ip()));
        }
    }
    Ok(addrs)
}

/// Refuse `url` before any connection is made. `resolve` asks DNS for the
/// host's addresses; the fixture path passes `false` so tests never resolve
/// a name. The scheme is checked regardless of `PI_WEB_FETCH_ALLOW_PRIVATE`.
fn guard(url: &Url, resolve: bool) -> Result<(), String> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!(
            "web_fetch refused {url}: only http and https can be fetched, not {}",
            url.scheme()
        ));
    }
    if allow_private() {
        return Ok(());
    }
    let refused = |why: String| Err(format!("web_fetch refused {url}: {why}"));
    match url.host() {
        None => refused("no host".into()),
        Some(Host::Ipv4(ip)) => match ipv4_refusal(ip) {
            Some(why) => refused(why.into()),
            None => Ok(()),
        },
        Some(Host::Ipv6(ip)) => match ipv6_refusal(ip) {
            Some(why) => refused(why.into()),
            None => Ok(()),
        },
        Some(Host::Domain(name)) => {
            if let Some(why) = local_name_refusal(name) {
                return refused(why.into());
            }
            if resolve {
                let port = url.port_or_known_default().unwrap_or(80);
                if let Err(why) = resolved_refusal(name, port) {
                    return refused(why);
                }
            }
            Ok(())
        }
    }
}

/// The resolver the live agent connects through: it re-applies the address
/// guard to the addresses actually dialled, so a name that changes its
/// answer between the pre-flight check and the connection (DNS rebinding)
/// is still refused.
fn guarded_resolve(netloc: &str) -> std::io::Result<Vec<SocketAddr>> {
    let (host, port) = netloc
        .rsplit_once(':')
        .and_then(|(host, port)| port.parse::<u16>().ok().map(|port| (host, port)))
        .unwrap_or((netloc, 80));
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if allow_private() {
        return netloc.to_socket_addrs().map(Iterator::collect);
    }
    resolved_refusal(host, port)
        .map_err(|why| std::io::Error::new(std::io::ErrorKind::PermissionDenied, why))
}

fn build_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(TIMEOUT)
        .redirects(0)
        .user_agent(USER_AGENT)
        .resolver(guarded_resolve)
        .build()
}

/// Read at most `cap` bytes; the flag says whether more was left behind.
fn read_capped(reader: impl Read, cap: u64) -> std::io::Result<(Vec<u8>, bool)> {
    let mut bytes = Vec::new();
    reader.take(cap + 1).read_to_end(&mut bytes)?;
    let over = bytes.len() as u64 > cap;
    if over {
        bytes.truncate(cap as usize);
    }
    Ok((bytes, over))
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
    /// The body hit `BODY_CAP` and the rest was not read.
    truncated_body: bool,
}

/// One round trip: the page, or where the server sent us instead.
enum Hop {
    Page(Fetched),
    Redirect(String),
}

/// Follow redirects by hand so every hop passes the guard. The fixture
/// (`PI_WEB_FETCH_FIXTURE`) answers each hop by URL: an entry with a 3xx
/// `status` and a `location` redirects, `truncatedBody: true` stands in for
/// a body that hit the cap, and no name is ever resolved.
fn fetch(url: &Url) -> Result<Fetched, String> {
    let fixtures = fixture("PI_WEB_FETCH_FIXTURE");
    let agent = match fixtures {
        Some(_) => None,
        None if network_disabled() => {
            return Err("network disabled (PI_OFFLINE / PI_DISABLE_NETWORK)".into());
        }
        None => Some(build_agent()),
    };
    let mut current = url.clone();
    let mut hops = 0usize;
    loop {
        guard(&current, agent.is_some())?;
        let hop = match (&fixtures, &agent) {
            (Some(map), _) => fixture_hop(map, &current)?,
            (None, Some(agent)) => live_hop(agent, &current)?,
            (None, None) => unreachable!("a fetch has a fixture or an agent"),
        };
        let location = match hop {
            Hop::Page(page) => return Ok(page),
            Hop::Redirect(location) => location,
        };
        hops += 1;
        if hops > MAX_REDIRECTS {
            return Err(format!(
                "{url} redirected more than {MAX_REDIRECTS} times (last to {location})"
            ));
        }
        let next = current.join(&location).map_err(|err| {
            format!("{current} redirected to `{location}`, which is not a URL: {err}")
        })?;
        if davinci_ai::trace::enabled() {
            davinci_ai::trace::log(&format!("web_fetch redirect {current} → {next}"));
        }
        current = next;
    }
}

fn fixture_hop(map: &Value, url: &Url) -> Result<Hop, String> {
    let hit = map
        .get(url.as_str())
        .or_else(|| map.get(url.as_str().trim_end_matches('/')))
        .ok_or_else(|| format!("fixture has no entry for {url}"))?;
    let status = hit.get("status").and_then(Value::as_u64).unwrap_or(200) as u16;
    if let Some(location) = redirect_location(status, hit.get("location").and_then(Value::as_str)) {
        return Ok(Hop::Redirect(location));
    }
    Ok(Hop::Page(Fetched {
        status,
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
        truncated_body: hit
            .get("truncatedBody")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }))
}

fn redirect_location(status: u16, location: Option<&str>) -> Option<String> {
    if !matches!(status, 301 | 302 | 303 | 307 | 308) {
        return None;
    }
    location
        .map(str::trim)
        .filter(|location| !location.is_empty())
        .map(str::to_string)
}

fn live_hop(agent: &ureq::Agent, url: &Url) -> Result<Hop, String> {
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
    if let Some(location) = redirect_location(status, response.header("location")) {
        return Ok(Hop::Redirect(location));
    }
    let content_type = response
        .header("content-type")
        .unwrap_or("application/octet-stream")
        .to_string();
    let (bytes, truncated_body) = read_capped(response.into_reader(), BODY_CAP)
        .map_err(|err| format!("could not read {url}: {err}"))?;
    if davinci_ai::trace::enabled() {
        davinci_ai::trace::log(&format!(
            "web_fetch GET {url} → {status} {content_type} {}k{}",
            bytes.len() / 1024,
            if truncated_body { " (capped)" } else { "" }
        ));
    }
    Ok(Hop::Page(Fetched {
        status,
        content_type,
        body: String::from_utf8_lossy(&bytes).into_owned(),
        final_url: url.to_string(),
        truncated_body,
    }))
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
    let body = match title {
        Some(title) if !text.starts_with(&title) => format!("{title}\n\n{text}"),
        _ => text,
    };
    // The read cap (2000 lines / 50 KB) applies unless the caller chose its
    // own `maxChars`, which then stands alone.
    let max_chars = input
        .get("maxChars")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .filter(|n| *n > 0);
    let (mut content, truncation) = match max_chars {
        Some(max) => {
            let total = body.chars().count();
            let content = if total > max {
                body.chars().take(max).collect::<String>()
                    + &format!("\n\n[truncated to {max} characters]")
            } else {
                body
            };
            (
                content,
                json!({
                    "truncated": total > max,
                    "truncatedBy": if total > max { Some("maxChars") } else { None },
                    "totalChars": total,
                    "outputChars": total.min(max),
                    "maxChars": max,
                }),
            )
        }
        None => {
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
            (content, truncation)
        }
    };
    if fetched.truncated_body {
        content.push_str(&format!(
            "\n\n[Body cut at {BODY_CAP_LABEL}; the page is longer than web_fetch reads.]"
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
            "truncatedBody": fetched.truncated_body,
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
    if davinci_ai::trace::enabled() {
        davinci_ai::trace::log(&format!(
            "web_search duckduckgo `{query}` {}k",
            html.len() / 1024
        ));
    }
    let mut results = duckduckgo_results(&html);
    if results.is_empty() {
        if let Some(reason) = duckduckgo_block(&html) {
            return Err(format!(
                "DuckDuckGo answered `{query}` with a {reason} instead of results; \
                 retry later or set BRAVE_API_KEY to search through Brave"
            ));
        }
    }
    results.truncate(limit);
    Ok(("duckduckgo".into(), results))
}

/// DuckDuckGo's HTML endpoint sometimes answers automated traffic with an
/// "anomaly" page — a challenge instead of a result list. A page with no
/// `result__body` at all that carries one of those markers is that, not an
/// empty search.
pub fn duckduckgo_block(html: &str) -> Option<&'static str> {
    if html.contains("result__body") {
        return None;
    }
    let lower = html.to_ascii_lowercase();
    if lower.contains("anomaly-modal") || lower.contains("anomaly_modal") {
        Some("bot check (anomaly page)")
    } else if lower.contains("bots use duckduckgo too") || lower.contains("challenge-form") {
        Some("bot check (challenge page)")
    } else if lower.contains("captcha") {
        Some("bot check (captcha)")
    } else if lower.contains("rate limit") || lower.contains("too many requests") {
        Some("rate limit page")
    } else {
        None
    }
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

    /// Point `PI_WEB_FETCH_FIXTURE` at `map` for the rest of the scope.
    fn fixture_env(dir: &std::path::Path, map: Value) {
        let path = dir.join("fetch.json");
        std::fs::write(&path, serde_json::to_string(&map).unwrap()).unwrap();
        std::env::set_var("PI_WEB_FETCH_FIXTURE", &path);
    }

    #[test]
    fn addresses_of_this_machine_and_its_networks_are_refused() {
        let refused = |ip: &str| ip_refusal(ip.parse().unwrap());
        assert_eq!(refused("127.0.0.1"), Some("loopback address"));
        assert_eq!(refused("127.8.9.10"), Some("loopback address"));
        assert_eq!(refused("::1"), Some("loopback address"));
        assert_eq!(refused("10.1.2.3"), Some("private address"));
        assert_eq!(refused("172.16.0.1"), Some("private address"));
        assert_eq!(refused("172.31.255.255"), Some("private address"));
        assert_eq!(refused("192.168.1.1"), Some("private address"));
        assert_eq!(refused("fc00::1"), Some("private address"));
        assert_eq!(refused("fdab::1"), Some("private address"));
        assert_eq!(refused("169.254.169.254"), Some("link-local address"));
        assert_eq!(refused("fe80::1"), Some("link-local address"));
        assert_eq!(refused("0.0.0.0"), Some("unspecified address"));
        assert_eq!(refused("::"), Some("unspecified address"));
        assert_eq!(refused("224.0.0.1"), Some("multicast address"));
        assert_eq!(refused("ff02::1"), Some("multicast address"));
        assert_eq!(refused("255.255.255.255"), Some("broadcast address"));
        assert_eq!(refused("::ffff:127.0.0.1"), Some("loopback address"));
        assert_eq!(refused("::ffff:10.0.0.1"), Some("private address"));
        assert_eq!(
            refused("::ffff:169.254.169.254"),
            Some("link-local address")
        );
        assert_eq!(refused("::ffff:192.168.0.1"), Some("private address"));
        assert_eq!(refused("93.184.216.34"), None);
        assert_eq!(refused("172.32.0.1"), None);
        assert_eq!(refused("2606:2800:220:1:248:1893:25c8:1946"), None);
        assert_eq!(refused("::ffff:93.184.216.34"), None);

        assert_eq!(local_name_refusal("localhost"), Some("local hostname"));
        assert_eq!(local_name_refusal("LocalHost."), Some("local hostname"));
        assert_eq!(local_name_refusal("api.localhost"), Some("local hostname"));
        assert_eq!(local_name_refusal("printer.local"), Some("mDNS hostname"));
        assert_eq!(local_name_refusal("localhost.example.com"), None);
        assert_eq!(local_name_refusal("example.com"), None);

        // Literals and local names are judged without DNS.
        let judge = |raw: &str| guard(&Url::parse(raw).unwrap(), false);
        assert!(judge("https://example.com/").is_ok());
        assert!(judge("http://8.8.8.8/").is_ok());
        let err = judge("http://169.254.169.254/latest/meta-data/").unwrap_err();
        assert_eq!(
            err,
            "web_fetch refused http://169.254.169.254/latest/meta-data/: link-local address"
        );
        let err = judge("http://[::1]:8080/").unwrap_err();
        assert!(err.ends_with("loopback address"), "{err}");
        let err = judge("http://[::ffff:10.0.0.1]/").unwrap_err();
        assert!(err.ends_with("private address"), "{err}");
        // The url crate folds numeric host forms into an IPv4 literal.
        let err = judge("http://2130706433/").unwrap_err();
        assert!(err.ends_with("loopback address"), "{err}");
        let err = judge("http://0x7f000001/").unwrap_err();
        assert!(err.ends_with("loopback address"), "{err}");
        let err = judge("ftp://example.com/").unwrap_err();
        assert!(err.contains("only http and https"), "{err}");
    }

    #[test]
    fn fetch_refuses_loopback_link_local_private_and_localhost() {
        let _guard = env_lock();
        let dir = tempfile::tempdir().unwrap();
        // Entries exist for every target so a refusal can only come from
        // the guard, never from a missing fixture.
        fixture_env(
            dir.path(),
            json!({
                "http://127.0.0.1/": {"contentType": "text/plain", "body": "secret"},
                "http://169.254.169.254/": {"contentType": "text/plain", "body": "secret"},
                "http://10.0.0.5/": {"contentType": "text/plain", "body": "secret"},
                "http://192.168.1.1/": {"contentType": "text/plain", "body": "secret"},
                "http://localhost:3000/": {"contentType": "text/plain", "body": "secret"},
                "http://app.localhost/": {"contentType": "text/plain", "body": "secret"},
                "http://[::1]/": {"contentType": "text/plain", "body": "secret"},
                "http://[::ffff:127.0.0.1]/": {"contentType": "text/plain", "body": "secret"},
                "https://example.com/": {"contentType": "text/plain", "body": "public"}
            }),
        );
        let refused = |raw: &str| fetch_tool(&json!({"url": raw})).unwrap_err();
        assert_eq!(
            refused("http://127.0.0.1/"),
            "web_fetch refused http://127.0.0.1/: loopback address"
        );
        assert_eq!(
            refused("http://169.254.169.254/"),
            "web_fetch refused http://169.254.169.254/: link-local address"
        );
        assert_eq!(
            refused("http://10.0.0.5/"),
            "web_fetch refused http://10.0.0.5/: private address"
        );
        assert_eq!(
            refused("192.168.1.1"),
            "web_fetch refused https://192.168.1.1/: private address"
        );
        assert_eq!(
            refused("http://localhost:3000/"),
            "web_fetch refused http://localhost:3000/: local hostname"
        );
        assert_eq!(
            refused("http://app.localhost/"),
            "web_fetch refused http://app.localhost/: local hostname"
        );
        assert_eq!(
            refused("http://[::1]/"),
            "web_fetch refused http://[::1]/: loopback address"
        );
        let mapped = refused("http://[::ffff:127.0.0.1]/");
        assert!(
            mapped.starts_with("web_fetch refused http://[::ffff:")
                && mapped.ends_with(": loopback address"),
            "{mapped}"
        );
        let public = fetch_tool(&json!({"url": "https://example.com/"})).unwrap();
        assert_eq!(public.content, "public");

        // The escape hatch for local development servers.
        std::env::set_var("PI_WEB_FETCH_ALLOW_PRIVATE", "1");
        let local = fetch_tool(&json!({"url": "http://localhost:3000/"})).unwrap();
        assert_eq!(local.content, "secret");
        let scheme = fetch_tool(&json!({"url": "gopher://localhost/"})).unwrap_err();
        assert!(scheme.contains("only http and https"), "{scheme}");
        std::env::remove_var("PI_WEB_FETCH_ALLOW_PRIVATE");
        std::env::remove_var("PI_WEB_FETCH_FIXTURE");
    }

    #[test]
    fn every_redirect_hop_passes_the_guard() {
        let _guard = env_lock();
        let dir = tempfile::tempdir().unwrap();
        fixture_env(
            dir.path(),
            json!({
                // A public chain: absolute, then relative, then the page.
                "https://a.example/start": {"status": 301, "location": "https://b.example/one"},
                "https://b.example/one": {"status": 302, "location": "/two"},
                "https://b.example/two": {"contentType": "text/plain", "body": "landed"},
                // Hop 2 turns inward.
                "https://a.example/trap": {"status": 302, "location": "https://b.example/hop"},
                "https://b.example/hop": {"status": 307, "location": "http://10.0.0.5/admin"},
                "http://10.0.0.5/admin": {"contentType": "text/plain", "body": "secret"},
                // A hop to the metadata service, by relative-looking scheme swap.
                "https://a.example/meta": {"status": 303, "location": "http://169.254.169.254/latest/"},
                // A hop off http.
                "https://a.example/file": {"status": 302, "location": "file:///etc/passwd"},
                // A loop.
                "https://a.example/loop": {"status": 302, "location": "https://a.example/loop"},
                // A 3xx without a location is a page.
                "https://a.example/moved": {"status": 304, "contentType": "text/plain", "body": "unchanged"}
            }),
        );
        let landed = fetch_tool(&json!({"url": "https://a.example/start"})).unwrap();
        assert_eq!(landed.content, "landed");
        let details = landed.details.unwrap();
        assert_eq!(details["url"], "https://a.example/start");
        assert_eq!(details["finalUrl"], "https://b.example/two");
        assert_eq!(details["status"], 200);

        let trap = fetch_tool(&json!({"url": "https://a.example/trap"})).unwrap_err();
        assert_eq!(
            trap,
            "web_fetch refused http://10.0.0.5/admin: private address"
        );
        let meta = fetch_tool(&json!({"url": "https://a.example/meta"})).unwrap_err();
        assert_eq!(
            meta,
            "web_fetch refused http://169.254.169.254/latest/: link-local address"
        );
        let file = fetch_tool(&json!({"url": "https://a.example/file"})).unwrap_err();
        assert!(file.contains("only http and https"), "{file}");
        let looped = fetch_tool(&json!({"url": "https://a.example/loop"})).unwrap_err();
        assert!(looped.contains("redirected more than 5 times"), "{looped}");
        let moved = fetch_tool(&json!({"url": "https://a.example/moved"})).unwrap();
        assert_eq!(moved.content, "unchanged");
        std::env::remove_var("PI_WEB_FETCH_FIXTURE");
    }

    #[test]
    fn a_body_that_hits_the_cap_is_marked() {
        let (bytes, over) = read_capped(std::io::Cursor::new(b"0123456789"), 10).unwrap();
        assert_eq!(bytes, b"0123456789");
        assert!(!over);
        let (bytes, over) = read_capped(std::io::Cursor::new(b"0123456789x"), 10).unwrap();
        assert_eq!(bytes, b"0123456789");
        assert!(over);

        let _guard = env_lock();
        let dir = tempfile::tempdir().unwrap();
        fixture_env(
            dir.path(),
            json!({
                "https://example.com/big": {"contentType": "text/plain", "body": "head of a huge page", "truncatedBody": true},
                "https://example.com/small": {"contentType": "text/plain", "body": "whole"}
            }),
        );
        let big = fetch_tool(&json!({"url": "https://example.com/big"})).unwrap();
        assert!(
            big.content
                .starts_with("head of a huge page\n\n[Body cut at 10 MB;"),
            "{}",
            big.content
        );
        assert_eq!(big.details.unwrap()["truncatedBody"], true);
        let small = fetch_tool(&json!({"url": "https://example.com/small"})).unwrap();
        assert_eq!(small.content, "whole");
        assert_eq!(small.details.unwrap()["truncatedBody"], false);
        std::env::remove_var("PI_WEB_FETCH_FIXTURE");
    }

    #[test]
    fn max_chars_replaces_the_read_cap() {
        let _guard = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let long = (1..=3000).fold(String::new(), |mut text, n| {
            text.push_str(&format!("line {n}\n"));
            text
        });
        fixture_env(
            dir.path(),
            json!({
                "https://example.com/long": {"contentType": "text/plain", "body": long}
            }),
        );
        // Without maxChars the read cap of 2000 lines applies.
        let capped = fetch_tool(&json!({"url": "https://example.com/long"})).unwrap();
        assert!(
            capped.content.contains("[Showing 2000 of 3000 lines."),
            "{}",
            &capped.content[capped.content.len() - 200..]
        );
        let truncation = &capped.details.unwrap()["truncation"];
        assert_eq!(truncation["truncatedBy"], "lines");

        // With maxChars only that cut applies: all 3000 lines come through.
        let whole =
            fetch_tool(&json!({"url": "https://example.com/long", "maxChars": 100_000})).unwrap();
        assert!(
            whole.content.ends_with("line 3000\n"),
            "{}",
            &whole.content[whole.content.len() - 60..]
        );
        assert!(!whole.content.contains("[Showing"), "{}", whole.content);
        let truncation = &whole.details.unwrap()["truncation"];
        assert_eq!(truncation["truncated"], false);
        assert_eq!(truncation["maxChars"], 100_000);
        assert_eq!(truncation["totalChars"], long.chars().count());

        let cut = fetch_tool(&json!({"url": "https://example.com/long", "maxChars": 7})).unwrap();
        assert_eq!(cut.content, "line 1\n\n\n[truncated to 7 characters]");
        let truncation = &cut.details.unwrap()["truncation"];
        assert_eq!(truncation["truncatedBy"], "maxChars");
        assert_eq!(truncation["outputChars"], 7);
        std::env::remove_var("PI_WEB_FETCH_FIXTURE");
    }

    #[test]
    fn a_duckduckgo_bot_check_is_named_not_mistaken_for_no_results() {
        const ANOMALY: &str = r#"<!DOCTYPE html><html><head><title>DuckDuckGo</title></head>
<body><div class="anomaly-modal__modal"><div class="anomaly-modal__title">
Unfortunately, bots use DuckDuckGo too.</div><p>Please complete the following
challenge to confirm this search was made by a human.</p>
<form class="challenge-form" action="/html/"></form></div></body></html>"#;
        assert_eq!(duckduckgo_block(ANOMALY), Some("bot check (anomaly page)"));
        assert!(duckduckgo_results(ANOMALY).is_empty());
        // A genuinely empty result list is not a block.
        const EMPTY: &str = r#"<div class="results"><div class="no-results">
No results.</div></div>"#;
        assert_eq!(duckduckgo_block(EMPTY), None);
        // A page with results is never a block, whatever else it says.
        assert_eq!(duckduckgo_block(DDG), None);
    }
}
