//! The host half of the Discover view in `/plugins`, `/skills` and `/mcp`.
//!
//! Local catalogs answer as the user types, from a cache filled once per
//! sheet. Online directories (skills.sh, the MCP Registry) are asked on a
//! background thread after a short pause in typing; the answer is merged on
//! the next tick, and only if it still belongs to the current query.

use davinci_coding_agent::plugins::discover::{self, Kind, Listing};
use davinci_tui::davinci::model::{ExtensionRow, ExtensionTab, Model};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;
use std::time::Duration;

/// Typing pause before an online search goes out.
const DEBOUNCE: Duration = Duration::from_millis(300);

type Reply = (u64, Result<Vec<Listing>, String>);

struct State {
    generation: u64,
    /// Offline listings per kind, read once per sheet.
    local: BTreeMap<u8, Vec<Listing>>,
    /// Every listing on screen, by key, so install gets its payload.
    shown: BTreeMap<String, Listing>,
    /// The local half of the current results, kept while the online half is out.
    pending_local: Vec<Listing>,
    /// Registry servers already in the user `mcp.json`, by registry name.
    registry_installed: BTreeSet<String>,
    sender: Sender<Reply>,
    receiver: Receiver<Reply>,
}

fn state() -> &'static Mutex<State> {
    static STATE: std::sync::OnceLock<Mutex<State>> = std::sync::OnceLock::new();
    STATE.get_or_init(|| {
        let (sender, receiver) = channel();
        Mutex::new(State {
            generation: 0,
            local: BTreeMap::new(),
            shown: BTreeMap::new(),
            pending_local: Vec::new(),
            registry_installed: BTreeSet::new(),
            sender,
            receiver,
        })
    })
}

fn lock() -> std::sync::MutexGuard<'static, State> {
    state().lock().unwrap_or_else(|err| err.into_inner())
}

pub(super) fn kind(tab: ExtensionTab) -> Kind {
    match tab {
        ExtensionTab::Plugins => Kind::Plugin,
        ExtensionTab::Skills => Kind::Skill,
        ExtensionTab::Mcp => Kind::Mcp,
    }
}

fn slot(kind: Kind) -> u8 {
    match kind {
        Kind::Plugin => 0,
        Kind::Skill => 1,
        Kind::Mcp => 2,
    }
}

/// Forget cached catalogs: the sheet reopened, or something was installed.
pub(super) fn invalidate() {
    let mut state = lock();
    state.local.clear();
    state.generation += 1;
}

fn online_source(kind: Kind) -> Option<&'static str> {
    match kind {
        Kind::Plugin => None,
        Kind::Skill => Some("skills.sh"),
        Kind::Mcp => Some("the MCP Registry"),
    }
}

fn thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

pub(super) fn row(listing: &Listing, configured: &BTreeSet<String>) -> ExtensionRow {
    let mut detail = listing.description.lines().next().unwrap_or("").to_string();
    if detail.chars().count() > 160 {
        detail = format!("{}…", detail.chars().take(160).collect::<String>());
    }
    ExtensionRow {
        key: listing.key.clone(),
        title: listing.title.clone(),
        status: if listing.installed {
            "installed".into()
        } else {
            listing.source.clone()
        },
        detail,
        // A registry server says what it runs or contacts and which
        // variables it reads, before the second enter installs it.
        note: match (&listing.payload, listing.key.starts_with("registry:")) {
            (Some(server), true) => Some(discover::mcp_preview(server, configured)),
            _ => listing
                .installs
                .map(|count| format!("{} installs", thousands(count))),
        },
        ..ExtensionRow::default()
    }
}

/// Merge local and online listings: local first, then online ones whose
/// title no local listing already shows.
fn merge(local: &[Listing], remote: &[Listing]) -> Vec<Listing> {
    let mut out = local.to_vec();
    for listing in remote {
        if !out.iter().any(|known| known.title == listing.title) {
            out.push(listing.clone());
        }
    }
    out
}

fn show(model: &mut Model, listings: &[Listing], state: &mut State) {
    // A registry server whose name is already configured is installed.
    let configured: BTreeSet<String> = model
        .extension_manager
        .as_ref()
        .map(|sheet| sheet.mcp.iter().map(|row| row.key.clone()).collect())
        .unwrap_or_default();
    let listings: Vec<Listing> = listings
        .iter()
        .cloned()
        .map(|mut listing| {
            if let Some(server) = listing
                .payload
                .as_ref()
                .filter(|_| listing.key.starts_with("registry:"))
            {
                let name = server.get("name").and_then(|v| v.as_str()).unwrap_or("");
                listing.installed = state.registry_installed.contains(name);
            }
            listing
        })
        .collect();
    let listings = listings.as_slice();
    state.shown = listings
        .iter()
        .map(|listing| (listing.key.clone(), listing.clone()))
        .collect();
    if let Some(sheet) = model.extension_manager.as_mut() {
        let selected = sheet.discover.current().map(|row| row.key.clone());
        sheet.discover.results = listings
            .iter()
            .map(|listing| row(listing, &configured))
            .collect();
        // Keep the selection on the same result when it is still listed.
        sheet.discover.selected = selected
            .and_then(|key| sheet.discover.results.iter().position(|row| row.key == key))
            .unwrap_or(0);
    }
}

/// Answer the query now from local catalogs; ask online directories later.
pub(super) fn search(
    model: &mut Model,
    tab: ExtensionTab,
    query: &str,
    agent_dir: &Path,
    mcp_file: &Path,
) {
    let kind = kind(tab);
    let mut state = lock();
    state.registry_installed = discover::registry_installs(mcp_file).into_keys().collect();
    state.generation += 1;
    let generation = state.generation;
    let all = state
        .local
        .entry(slot(kind))
        .or_insert_with(|| discover::all_local(kind, agent_dir))
        .clone();
    let local = discover::filter(query, &all);
    state.pending_local = local.clone();
    show(model, &local, &mut state);
    let asks_online = match kind {
        Kind::Skill => query.trim().chars().count() >= 2,
        Kind::Mcp => true,
        Kind::Plugin => false,
    };
    if let Some(sheet) = model.extension_manager.as_mut() {
        sheet.discover.searching = asks_online;
        sheet.discover.message = match (kind, asks_online) {
            (Kind::Skill, false) => Some(format!(
                "{} skills in your marketplaces. Type 2+ letters to also search skills.sh.",
                all.len()
            )),
            (Kind::Plugin, _) if all.is_empty() => {
                Some("No marketplaces yet: add one in the Marketplaces view (→).".to_string())
            }
            (Kind::Plugin, _) => Some(format!("{} plugins in your marketplaces.", all.len())),
            _ => None,
        };
    }
    if !asks_online {
        return;
    }
    let sender = state.sender.clone();
    drop(state);
    let query = query.to_string();
    let agent_dir: PathBuf = agent_dir.to_path_buf();
    std::thread::spawn(move || {
        std::thread::sleep(DEBOUNCE);
        if lock().generation != generation {
            return;
        }
        let result = discover::remote(kind, &agent_dir, &query);
        let _ = sender.send((generation, result));
    });
}

/// Merge an online answer that arrived since the last tick. True when the
/// sheet changed.
pub(super) fn poll(model: &mut Model, tab: ExtensionTab) -> bool {
    let mut state = lock();
    let mut changed = false;
    while let Ok((generation, result)) = state.receiver.try_recv() {
        if generation != state.generation {
            continue;
        }
        let source = online_source(kind(tab)).unwrap_or("online");
        match result {
            Ok(remote) => {
                let merged = merge(&state.pending_local, &remote);
                show(model, &merged, &mut state);
                if let Some(sheet) = model.extension_manager.as_mut() {
                    sheet.discover.message = (remote.is_empty()
                        && sheet.discover.results.is_empty())
                    .then(|| format!("Nothing on {source} matches."));
                }
            }
            Err(error) => {
                if let Some(sheet) = model.extension_manager.as_mut() {
                    sheet.discover.message = Some(format!("Could not reach {source}: {error}"));
                }
            }
        }
        if let Some(sheet) = model.extension_manager.as_mut() {
            sheet.discover.searching = false;
        }
        changed = true;
    }
    changed
}

/// The listing behind a result key on screen.
pub(super) fn listing(key: &str) -> Option<Listing> {
    lock().shown.get(key).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing(title: &str, installs: Option<u64>) -> Listing {
        Listing {
            key: format!("k:{title}"),
            title: title.into(),
            description: "first line\nsecond".into(),
            source: "src".into(),
            installed: false,
            installs,
            payload: None,
        }
    }

    #[test]
    fn rows_show_source_or_installed_and_install_counts() {
        let none = BTreeSet::new();
        let shown = row(&listing("pdf", Some(205751)), &none);
        assert_eq!(shown.status, "src");
        assert_eq!(shown.detail, "first line");
        assert_eq!(shown.note.as_deref(), Some("205,751 installs"));
        let installed = row(
            &Listing {
                installed: true,
                ..listing("pdf", None)
            },
            &none,
        );
        assert_eq!(installed.status, "installed");
        assert_eq!(installed.note, None);
    }

    #[test]
    fn online_results_never_repeat_a_local_title() {
        let merged = merge(
            &[listing("pdf", None)],
            &[listing("pdf", Some(1)), listing("docx", Some(2))],
        );
        let titles: Vec<_> = merged.iter().map(|l| l.title.as_str()).collect();
        assert_eq!(titles, ["pdf", "docx"]);
        assert_eq!(merged[0].installs, None, "the local listing wins");
    }

    #[test]
    fn thousands_groups_digits() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(1234567), "1,234,567");
    }
}
