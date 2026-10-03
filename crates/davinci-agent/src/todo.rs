//! The model's own ledger: a list of steps it keeps current with the `todo`
//! tool (the `TodoWrite` shape models already know). No TypeScript
//! counterpart; phase 3 spec, "Todo ledger".
//!
//! The list lives on the agent behind an `Arc<Mutex>` so the davinci shell
//! can draw it as the STUDIO ledger without an event, and it is written to
//! the session as a `todo` custom entry after every change so a resumed
//! session opens on the plan it closed on.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Where a step stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TodoStatus {
    #[default]
    Pending,
    Active,
    Done,
}

impl TodoStatus {
    /// `pending | active | done`, with the synonyms the shape's other users
    /// send: `in_progress`, `completed`, `todo`.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "pending" | "todo" | "open" | "queued" => Some(Self::Pending),
            "active" | "in_progress" | "doing" | "current" => Some(Self::Active),
            "done" | "completed" | "complete" | "finished" => Some(Self::Done),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Done => "done",
        }
    }

    /// The glyph the ledger draws (design.md §4): `✓` done, `◉` active,
    /// `○` pending.
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Pending => "○",
            Self::Active => "◉",
            Self::Done => "✓",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoItem {
    pub text: String,
    #[serde(default)]
    pub status: TodoStatus,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoList {
    pub items: Vec<TodoItem>,
}

impl TodoList {
    /// The `todo` tool's arguments: `{ items: [{ text, status }] }` replaces
    /// the whole list. A bare array is taken as the items.
    pub fn from_args(args: &Value) -> Result<Self, String> {
        let raw = match args.get("items") {
            Some(items) => items,
            None if args.is_array() => args,
            None => return Err("todo needs `items`: an array of { text, status }.".into()),
        };
        let Some(items) = raw.as_array() else {
            return Err("todo `items` must be an array of { text, status }.".into());
        };
        let mut list = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            let text = match item {
                Value::String(text) => text.trim().to_string(),
                other => other
                    .get("text")
                    .or_else(|| other.get("content"))
                    .or_else(|| other.get("title"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string(),
            };
            if text.is_empty() {
                return Err(format!("items[{index}] has no text."));
            }
            let status = match item.get("status").and_then(Value::as_str) {
                None => TodoStatus::Pending,
                Some(status) => TodoStatus::parse(status).ok_or_else(|| {
                    format!(
                        "items[{index}].status must be pending, active or done, not `{status}`."
                    )
                })?,
            };
            list.push(TodoItem { text, status });
        }
        Ok(Self { items: list })
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn done(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.status == TodoStatus::Done)
            .count()
    }

    pub fn active(&self) -> Option<&TodoItem> {
        self.items
            .iter()
            .find(|item| item.status == TodoStatus::Active)
    }

    /// The ledger as text — the tool's result and `/todo`'s reply:
    ///
    /// ```text
    /// 3 items · 1 done · 1 active
    /// ✓ read the parser
    /// ◉ add the notebook branch
    /// ○ run the tests
    /// ```
    pub fn render(&self) -> String {
        if self.items.is_empty() {
            return "(no items)".into();
        }
        let active = self
            .items
            .iter()
            .filter(|item| item.status == TodoStatus::Active)
            .count();
        let mut out = vec![format!(
            "{} · {} done · {active} active",
            plural(self.items.len(), "item"),
            self.done()
        )];
        out.extend(
            self.items
                .iter()
                .map(|item| format!("{} {}", item.status.glyph(), item.text)),
        );
        out.join("\n")
    }

    /// `1 of 3 done` for a tool line's summary.
    pub fn summary(&self) -> String {
        if self.items.is_empty() {
            return "cleared".into();
        }
        format!("{} of {} done", self.done(), self.items.len())
    }

    /// The session entry's payload, and its reverse.
    pub fn to_value(&self) -> Value {
        json!({ "items": self.items })
    }

    pub fn from_value(value: &Value) -> Option<Self> {
        serde_json::from_value(value.clone()).ok()
    }
}

fn plural(count: usize, unit: &str) -> String {
    if count == 1 {
        format!("1 {unit}")
    } else {
        format!("{count} {unit}s")
    }
}

/// The custom session entry type the ledger is stored under.
pub const TODO_ENTRY_TYPE: &str = "todo";

pub fn tool_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "items": {
                "type": "array",
                "description": "The whole list, in order. Sending it replaces the previous list.",
                "items": {
                    "type": "object",
                    "properties": {
                        "text": {"type": "string", "description": "The step, in a few words"},
                        "status": {"type": "string", "enum": ["pending", "active", "done"]}
                    },
                    "required": ["text", "status"]
                }
            }
        },
        "required": ["items"]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_replaced_whole_and_synonyms_are_understood() {
        let list = TodoList::from_args(&json!({"items": [
            {"text": "read the parser", "status": "completed"},
            {"text": "add the notebook branch", "status": "in_progress"},
            {"text": "run the tests"}
        ]}))
        .unwrap();
        assert_eq!(list.items.len(), 3);
        assert_eq!(list.items[0].status, TodoStatus::Done);
        assert_eq!(list.items[1].status, TodoStatus::Active);
        assert_eq!(list.items[2].status, TodoStatus::Pending);
        assert_eq!(
            list.render(),
            "3 items · 1 done · 1 active\n✓ read the parser\n◉ add the notebook branch\n○ run the tests"
        );
        assert_eq!(list.summary(), "1 of 3 done");
        assert_eq!(list.active().unwrap().text, "add the notebook branch");

        let bare =
            TodoList::from_args(&json!(["one", {"content": "two", "status": "done"}])).unwrap();
        assert_eq!(bare.items[1].text, "two");
        let empty = TodoList::from_args(&json!({"items": []})).unwrap();
        assert!(empty.is_empty());
        assert_eq!(empty.render(), "(no items)");
        assert_eq!(empty.summary(), "cleared");
    }

    #[test]
    fn bad_items_are_named() {
        let no_items = TodoList::from_args(&json!({})).unwrap_err();
        assert!(no_items.contains("needs `items`"));
        let blank = TodoList::from_args(&json!({"items": [{"text": "  "}]})).unwrap_err();
        assert_eq!(blank, "items[0] has no text.");
        let status =
            TodoList::from_args(&json!({"items": [{"text": "x", "status": "later"}]})).unwrap_err();
        assert!(status.contains("not `later`"));
    }

    #[test]
    fn the_ledger_round_trips_through_its_session_value() {
        let list =
            TodoList::from_args(&json!({"items": [{"text": "a", "status": "active"}]})).unwrap();
        let value = list.to_value();
        assert_eq!(value["items"][0]["status"], "active");
        assert_eq!(TodoList::from_value(&value).unwrap(), list);
    }
}
