use super::{JsExtensionResult, LoadCacheKey};
use serde_json::Value;
use std::borrow::Borrow;
use std::collections::VecDeque;

pub(super) struct LoadMemo {
    entries: VecDeque<(LoadCacheKey, JsExtensionResult, usize)>,
    bytes: usize,
    pub max_entries: usize,
    pub max_bytes: usize,
}
impl LoadMemo {
    pub const fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            bytes: 0,
            max_entries: 128,
            max_bytes: 8 * 1024 * 1024,
        }
    }
    pub fn clear(&mut self) {
        self.entries = VecDeque::new();
        self.bytes = 0;
    }
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn trim(&mut self) {
        while self.entries.len() > self.max_entries || self.bytes > self.max_bytes {
            if let Some((_, _, weight)) = self.entries.pop_front() {
                self.bytes -= weight;
            } else {
                break;
            }
        }
        if self.entries.is_empty() {
            self.entries = VecDeque::new();
        } else if self.entries.capacity() > self.entries.len().saturating_mul(2) {
            self.entries.shrink_to_fit();
        }
    }
    pub fn get(&mut self, key: &LoadCacheKey) -> Option<&JsExtensionResult> {
        let position = self.entries.iter().position(|(entry, _, _)| entry == key)?;
        let value = self.entries.remove(position)?;
        self.entries.push_back(value);
        self.entries.back().map(|(_, result, _)| result)
    }
    pub fn insert(&mut self, key: LoadCacheKey, result: impl Borrow<JsExtensionResult>) {
        let result = result.borrow();
        let mut removed = 0;
        self.entries.retain(|(other, _, weight)| {
            let keep = other != &key && !(other.0 == key.0 && other.1 != key.1);
            if !keep {
                removed += weight;
            }
            keep
        });
        self.bytes -= removed;
        let weight = key
            .0
            .capacity()
            .saturating_mul(2)
            .saturating_add(key.2.capacity())
            .saturating_add(std::mem::size_of::<(LoadCacheKey, JsExtensionResult, usize)>() * 2)
            .saturating_add(result_heap(result));
        if self.max_entries == 0 || weight > self.max_bytes {
            return;
        }
        self.bytes = self.bytes.saturating_add(weight);
        self.entries.push_back((key, (*result).clone(), weight));
        self.trim();
    }
}

fn sum(values: impl IntoIterator<Item = usize>) -> usize {
    values.into_iter().fold(0usize, usize::saturating_add)
}
fn text(value: &String) -> usize {
    value.capacity().saturating_add(16)
}
fn optional_text(value: &Option<String>) -> usize {
    value.as_ref().map_or(0, text)
}
fn vector<T>(values: &Vec<T>, heap: impl Fn(&T) -> usize) -> usize {
    values
        .capacity()
        .saturating_mul(std::mem::size_of::<T>())
        .saturating_add(16)
        .saturating_add(sum(values.iter().map(heap)))
}
fn value_heap(value: &Value) -> usize {
    match value {
        Value::String(value) => text(value),
        Value::Array(values) => vector(values, value_heap),
        // Conservative per-entry BTreeMap node allowance, including sparsely
        // populated nodes. Compact JSON bytes do not account for this heap.
        Value::Object(values) => sum(values.iter().map(|(key, value)| {
            1024usize
                .saturating_add(text(key))
                .saturating_add(value_heap(value))
        })),
        _ => 0,
    }
}
fn item_heap(item: &super::JsAutocompleteItem) -> usize {
    sum([
        text(&item.value),
        text(&item.label),
        optional_text(&item.description),
    ])
}
fn result_heap(result: &JsExtensionResult) -> usize {
    let strings = [
        &result.handlers,
        &result.unregistered_providers,
        &result.flags,
        &result.shortcuts,
        &result.message_renderers,
        &result.entry_renderers,
    ];
    sum([
        sum(strings.into_iter().map(|values| vector(values, text))),
        sum([
            &result.ui_calls,
            &result.session_calls,
            &result.event_emits,
            &result.updates,
        ]
        .into_iter()
        .map(|values| vector(values, value_heap))),
        sum([&result.current_theme, &result.result]
            .into_iter()
            .map(|value| value.as_ref().map_or(0, value_heap))),
        optional_text(&result.error),
        vector(&result.tools, |tool| {
            sum([
                text(&tool.name),
                text(&tool.description),
                optional_text(&tool.execution_mode),
                optional_text(&tool.render_shell),
                tool.parameters.as_ref().map_or(0, value_heap),
            ])
        }),
        vector(&result.providers, |provider| {
            text(&provider.name).saturating_add(value_heap(&provider.config))
        }),
        vector(&result.commands, |command| {
            sum([
                text(&command.name),
                text(&command.description),
                vector(&command.argument_items, item_heap),
            ])
        }),
        vector(&result.autocomplete_providers, |provider| {
            vector(&provider.trigger_characters, text)
                .saturating_add(vector(&provider.items, item_heap))
        }),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn heap_heavy_json_is_not_accounted_only_as_compact_serialized_bytes() {
        let mut memo = LoadMemo::new();
        memo.max_bytes = 4096;
        let result = JsExtensionResult {
            result: Some(serde_json::json!((0..64)
                .map(|_| serde_json::json!({"a":null}))
                .collect::<Vec<_>>())),
            ..Default::default()
        };
        memo.insert(("module".into(), None, "{}".into()), result);
        assert!(
            memo.is_empty(),
            "small JSON encoding hid large retained container allocations"
        );
    }
}
