//! Scoped-models selector matching TS `scoped-models-selector.ts`.

use crate::fuzzy::fuzzy_match;
use crate::render::Component;

pub type EnabledIds = Option<Vec<String>>;

pub fn is_enabled(enabled_ids: &EnabledIds, id: &str) -> bool {
    match enabled_ids {
        None => true,
        Some(ids) => ids.iter().any(|item| item == id),
    }
}

pub fn toggle(enabled_ids: EnabledIds, id: &str) -> EnabledIds {
    match enabled_ids {
        None => Some(vec![id.to_string()]),
        Some(ids) => {
            if let Some(index) = ids.iter().position(|item| item == id) {
                let mut next = ids;
                next.remove(index);
                Some(next)
            } else {
                let mut next = ids;
                next.push(id.to_string());
                Some(next)
            }
        }
    }
}

pub fn enable_all(
    enabled_ids: EnabledIds,
    all_ids: &[String],
    target_ids: Option<&[String]>,
) -> EnabledIds {
    let ids = enabled_ids?;
    let targets = target_ids.unwrap_or(all_ids);
    let mut result = ids;
    for id in targets {
        if !result.iter().any(|item| item == id) {
            result.push(id.clone());
        }
    }
    if result.len() == all_ids.len() && result.iter().all(|id| all_ids.contains(id)) {
        None
    } else {
        Some(result)
    }
}

pub fn clear_all(
    enabled_ids: EnabledIds,
    all_ids: &[String],
    target_ids: Option<&[String]>,
) -> EnabledIds {
    match enabled_ids {
        None => {
            if let Some(targets) = target_ids {
                Some(
                    all_ids
                        .iter()
                        .filter(|id| !targets.contains(id))
                        .cloned()
                        .collect(),
                )
            } else {
                Some(Vec::new())
            }
        }
        Some(ids) => {
            let targets: Vec<String> = target_ids
                .map(|items| items.to_vec())
                .unwrap_or_else(|| ids.clone());
            Some(ids.into_iter().filter(|id| !targets.contains(id)).collect())
        }
    }
}

pub fn move_id(enabled_ids: EnabledIds, id: &str, delta: isize) -> EnabledIds {
    let ids = enabled_ids?;
    let mut list = ids;
    let Some(index) = list.iter().position(|item| item == id) else {
        return Some(list);
    };
    let new_index = index as isize + delta;
    if new_index < 0 || new_index >= list.len() as isize {
        return Some(list);
    }
    list.swap(index, new_index as usize);
    Some(list)
}

pub fn get_sorted_ids(enabled_ids: &EnabledIds, all_ids: &[String]) -> Vec<String> {
    match enabled_ids {
        None => all_ids.to_vec(),
        Some(ids) => {
            let enabled: std::collections::HashSet<&String> = ids.iter().collect();
            let mut out = ids.clone();
            out.extend(all_ids.iter().filter(|id| !enabled.contains(id)).cloned());
            out
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedModel {
    pub provider: String,
    pub id: String,
    pub name: String,
}

impl ScopedModel {
    pub fn full_id(&self) -> String {
        format!("{}/{}", self.provider, self.id)
    }

    pub fn search_text(&self) -> String {
        format!("{} {} {}", self.id, self.provider, self.name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopedModelsAction {
    None,
    Change(EnabledIds),
    Persist(EnabledIds),
    Cancel,
}

#[derive(Debug, Clone)]
pub struct ScopedModelsSelector {
    pub models: Vec<ScopedModel>,
    pub enabled_ids: EnabledIds,
    pub selected: usize,
    pub query: String,
    pub dirty: bool,
    pub max_visible: usize,
    pub refresh_status: Option<String>,
}

impl ScopedModelsSelector {
    pub fn new(models: Vec<ScopedModel>, enabled_ids: EnabledIds) -> Self {
        Self {
            models,
            enabled_ids,
            selected: 0,
            query: String::new(),
            dirty: false,
            max_visible: 8,
            refresh_status: None,
        }
    }

    pub fn all_ids(&self) -> Vec<String> {
        self.models.iter().map(ScopedModel::full_id).collect()
    }

    pub fn filtered_ids(&self) -> Vec<String> {
        let sorted = get_sorted_ids(&self.enabled_ids, &self.all_ids());
        if self.query.is_empty() {
            return sorted;
        }
        sorted
            .into_iter()
            .filter(|id| {
                let text = self
                    .models
                    .iter()
                    .find(|model| model.full_id() == *id)
                    .map(ScopedModel::search_text)
                    .unwrap_or_else(|| id.clone());
                fuzzy_match(&self.query, &text).matches
            })
            .collect()
    }

    fn notify_change(&self) -> ScopedModelsAction {
        ScopedModelsAction::Change(self.enabled_ids.clone())
    }

    pub fn handle_key(&mut self, data: &str) -> ScopedModelsAction {
        let filtered = self.filtered_ids();
        match data {
            "\x1b[A" => {
                if !filtered.is_empty() {
                    self.selected = if self.selected == 0 {
                        filtered.len() - 1
                    } else {
                        self.selected - 1
                    };
                }
                ScopedModelsAction::None
            }
            "\x1b[B" => {
                if !filtered.is_empty() {
                    self.selected = if self.selected + 1 >= filtered.len() {
                        0
                    } else {
                        self.selected + 1
                    };
                }
                ScopedModelsAction::None
            }
            "\x1b[1;3A" => self.reorder(-1, &filtered),
            "\x1b[1;3B" => self.reorder(1, &filtered),
            "\r" | "\n" | " " => {
                if let Some(id) = filtered.get(self.selected) {
                    self.enabled_ids = toggle(self.enabled_ids.clone(), id);
                    self.dirty = true;
                    return self.notify_change();
                }
                ScopedModelsAction::None
            }
            "\x01" => {
                let targets = if self.query.is_empty() {
                    None
                } else {
                    Some(filtered)
                };
                self.enabled_ids = enable_all(
                    self.enabled_ids.clone(),
                    &self.all_ids(),
                    targets.as_deref(),
                );
                self.dirty = true;
                self.notify_change()
            }
            "\x18" => {
                let targets = if self.query.is_empty() {
                    None
                } else {
                    Some(filtered)
                };
                self.enabled_ids = clear_all(
                    self.enabled_ids.clone(),
                    &self.all_ids(),
                    targets.as_deref(),
                );
                self.dirty = true;
                self.notify_change()
            }
            "\x10" => {
                if let Some(id) = filtered.get(self.selected) {
                    if let Some(model) = self.models.iter().find(|model| model.full_id() == *id) {
                        let provider = model.provider.clone();
                        let provider_ids: Vec<String> = self
                            .models
                            .iter()
                            .filter(|item| item.provider == provider)
                            .map(ScopedModel::full_id)
                            .collect();
                        let all_on = provider_ids
                            .iter()
                            .all(|item| is_enabled(&self.enabled_ids, item));
                        self.enabled_ids = if all_on {
                            clear_all(
                                self.enabled_ids.clone(),
                                &self.all_ids(),
                                Some(&provider_ids),
                            )
                        } else {
                            enable_all(
                                self.enabled_ids.clone(),
                                &self.all_ids(),
                                Some(&provider_ids),
                            )
                        };
                        self.dirty = true;
                        return self.notify_change();
                    }
                }
                ScopedModelsAction::None
            }
            "\x13" => {
                self.dirty = false;
                ScopedModelsAction::Persist(self.enabled_ids.clone())
            }
            "\x03" => {
                if !self.query.is_empty() {
                    self.query.clear();
                    ScopedModelsAction::None
                } else {
                    ScopedModelsAction::Cancel
                }
            }
            "\x1b" => ScopedModelsAction::Cancel,
            "\x7f" | "\x08" => {
                self.query.pop();
                ScopedModelsAction::None
            }
            other if !other.chars().any(|ch| ch.is_control()) => {
                self.query.push_str(other);
                ScopedModelsAction::None
            }
            _ => ScopedModelsAction::None,
        }
    }

    fn reorder(&mut self, delta: isize, filtered: &[String]) -> ScopedModelsAction {
        if self.enabled_ids.is_none() {
            return ScopedModelsAction::None;
        }
        let Some(id) = filtered.get(self.selected).cloned() else {
            return ScopedModelsAction::None;
        };
        if !is_enabled(&self.enabled_ids, &id) {
            return ScopedModelsAction::None;
        }
        let current = self
            .enabled_ids
            .as_ref()
            .and_then(|ids| ids.iter().position(|item| item == &id));
        if let Some(index) = current {
            let new_index = index as isize + delta;
            if new_index >= 0
                && new_index < self.enabled_ids.as_ref().map(|ids| ids.len()).unwrap_or(0) as isize
            {
                self.enabled_ids = move_id(self.enabled_ids.clone(), &id, delta);
                self.dirty = true;
                self.selected = (self.selected as isize + delta) as usize;
                return self.notify_change();
            }
        }
        ScopedModelsAction::None
    }

    fn footer(&self) -> String {
        let all_ids = self.all_ids();
        let enabled_count = self
            .enabled_ids
            .as_ref()
            .map(|ids| ids.iter().filter(|id| all_ids.contains(id)).count())
            .unwrap_or(all_ids.len());
        let unavailable = self
            .enabled_ids
            .as_ref()
            .map(|ids| ids.iter().filter(|id| !all_ids.contains(id)).count())
            .unwrap_or(0);
        let count_text = if self.enabled_ids.is_none() {
            "all enabled".to_string()
        } else {
            let extra = if unavailable > 0 {
                format!(" · {unavailable} unavailable")
            } else {
                String::new()
            };
            format!("{enabled_count}/{} enabled{extra}", all_ids.len())
        };
        let mut line = format!(
            "  enter toggle · ctrl+a all · ctrl+x clear · ctrl+p provider · alt+up/alt+down reorder · ctrl+s save · {count_text}"
        );
        if self.dirty {
            line.push_str(" (unsaved)");
        }
        line
    }
}

impl Component for ScopedModelsSelector {
    fn render(&self, _width: usize) -> Vec<String> {
        let mut lines = vec![
            "Model Configuration".into(),
            "Session-only. ctrl+s to save to settings.".into(),
        ];
        if !self.query.is_empty() {
            lines.push(format!("/{query}", query = self.query));
        }
        let filtered = self.filtered_ids();
        if filtered.is_empty() {
            lines.push("  No matching models".into());
        } else {
            let start = self
                .selected
                .saturating_sub(self.max_visible / 2)
                .min(filtered.len().saturating_sub(self.max_visible));
            let end = (start + self.max_visible).min(filtered.len());
            let all_enabled = self.enabled_ids.is_none();
            for (offset, id) in filtered[start..end].iter().enumerate() {
                let index = start + offset;
                let model = self.models.iter().find(|model| model.full_id() == *id);
                let prefix = if index == self.selected { "→ " } else { "  " };
                let name = model.map(|m| m.id.as_str()).unwrap_or(id.as_str());
                let badge = match model {
                    Some(model) => format!(" [{}]", model.provider),
                    None => " [unavailable]".into(),
                };
                let status = match model {
                    Some(_) if all_enabled => String::new(),
                    Some(_) if is_enabled(&self.enabled_ids, id) => " ✓".into(),
                    _ => " ✗".into(),
                };
                lines.push(format!("{prefix}{name}{badge}{status}"));
            }
            if start > 0 || end < filtered.len() {
                lines.push(format!("  ({}/{})", self.selected + 1, filtered.len()));
            }
            if let Some(id) = filtered.get(self.selected) {
                let detail = self
                    .models
                    .iter()
                    .find(|model| model.full_id() == *id)
                    .map(|model| format!("  Model Name: {}", model.name))
                    .unwrap_or_else(|| "  Model unavailable".into());
                lines.push(detail);
            }
        }
        if let Some(status) = &self.refresh_status {
            lines.push(format!("  {status}"));
        }
        lines.push(self.footer());
        lines
    }

    fn handle_input(&mut self, data: &str) {
        let _ = self.handle_key(data);
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn models() -> Vec<ScopedModel> {
        vec![
            ScopedModel {
                provider: "faux".into(),
                id: "one".into(),
                name: "One".into(),
            },
            ScopedModel {
                provider: "faux".into(),
                id: "two".into(),
                name: "Two".into(),
            },
            ScopedModel {
                provider: "faux".into(),
                id: "three".into(),
                name: "Three".into(),
            },
        ]
    }

    #[test]
    fn enabled_id_helpers_match_ts() {
        let all = vec!["a".into(), "b".into(), "c".into()];
        assert!(is_enabled(&None, "a"));
        assert_eq!(toggle(None, "b"), Some(vec!["b".into()]));
        assert_eq!(
            toggle(Some(vec!["b".into()]), "b"),
            Some(Vec::<String>::new())
        );
        assert_eq!(enable_all(None, &all, None), None);
        assert_eq!(enable_all(Some(vec!["a".into()]), &all, None), None);
        assert_eq!(clear_all(None, &all, None), Some(Vec::<String>::new()));
        assert_eq!(
            move_id(Some(vec!["a".into(), "b".into(), "c".into()]), "a", 1),
            Some(vec!["b".into(), "a".into(), "c".into()])
        );
        assert_eq!(
            get_sorted_ids(&Some(vec!["c".into()]), &all),
            vec!["c", "a", "b"]
        );
    }

    #[test]
    fn selector_reorders_and_renders_ts_copy() {
        let ids: Vec<String> = models().iter().map(ScopedModel::full_id).collect();
        let mut selector = ScopedModelsSelector::new(models(), Some(ids.clone()));
        let rendered = selector.render(80).join("\n");
        assert!(rendered.contains("Model Configuration"));
        assert!(rendered.contains("Session-only."));
        assert!(rendered.contains("all enabled") || rendered.contains("3/3 enabled"));
        assert_eq!(
            selector.handle_key("\x1b[1;3B"),
            ScopedModelsAction::Change(Some(vec![ids[1].clone(), ids[0].clone(), ids[2].clone()]))
        );
        selector.query = "zzz".into();
        assert!(selector
            .render(80)
            .join("\n")
            .contains("No matching models"));
        assert_eq!(selector.handle_key("\x1b"), ScopedModelsAction::Cancel);
    }
}
