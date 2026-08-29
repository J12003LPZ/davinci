//! Extension UI context matching TypeScript `ExtensionUIContext` / `RpcExtensionUIRequest`.

use serde_json::{json, Value};
use std::collections::HashMap;

/// Every `RpcExtensionUIRequest.method` from `rpc-types.ts`.
pub const METHODS: &[&str] = &[
    "select",
    "confirm",
    "input",
    "editor",
    "notify",
    "setStatus",
    "setWidget",
    "setTitle",
    "set_editor_text",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiResponse {
    Value(String),
    Confirmed(bool),
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingKind {
    Select,
    Confirm,
    Input,
    Editor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Widget {
    pub lines: Vec<String>,
    pub placement: String,
}

#[derive(Debug, Clone, Default)]
pub struct ExtensionUiHost {
    pub statuses: HashMap<String, String>,
    pub widgets: HashMap<String, Widget>,
    pub title: Option<String>,
    pub editor_text: String,
    pub pending: HashMap<String, PendingKind>,
    pub notifications: Vec<(String, String)>,
}

impl ExtensionUiHost {
    pub fn request(method: &str, fields: Value) -> Value {
        let mut out = json!({
            "type": "extension_ui_request",
            "id": uuid::Uuid::new_v4().to_string(),
            "method": method,
        });
        if let (Some(dst), Some(obj)) = (out.as_object_mut(), fields.as_object()) {
            for (k, v) in obj {
                dst.insert(k.clone(), v.clone());
            }
        }
        out
    }

    fn enqueue_dialog(&mut self, kind: PendingKind, method: &str, fields: Value) -> Value {
        let req = Self::request(method, fields);
        if let Some(id) = req.get("id").and_then(|v| v.as_str()) {
            self.pending.insert(id.to_string(), kind);
        }
        req
    }

    pub fn select(&mut self, title: &str, options: &[String], timeout: Option<u64>) -> Value {
        let mut fields = json!({
            "title": title,
            "options": options,
        });
        if let Some(timeout) = timeout {
            fields["timeout"] = json!(timeout);
        }
        self.enqueue_dialog(PendingKind::Select, "select", fields)
    }

    pub fn confirm(&mut self, title: &str, message: &str, timeout: Option<u64>) -> Value {
        let mut fields = json!({
            "title": title,
            "message": message,
        });
        if let Some(timeout) = timeout {
            fields["timeout"] = json!(timeout);
        }
        self.enqueue_dialog(PendingKind::Confirm, "confirm", fields)
    }

    pub fn input(&mut self, title: &str, placeholder: Option<&str>, timeout: Option<u64>) -> Value {
        let mut fields = json!({ "title": title });
        if let Some(placeholder) = placeholder {
            fields["placeholder"] = json!(placeholder);
        }
        if let Some(timeout) = timeout {
            fields["timeout"] = json!(timeout);
        }
        self.enqueue_dialog(PendingKind::Input, "input", fields)
    }

    pub fn editor(&mut self, title: &str, prefill: Option<&str>) -> Value {
        let mut fields = json!({ "title": title });
        if let Some(prefill) = prefill {
            fields["prefill"] = json!(prefill);
            self.editor_text = prefill.to_string();
        }
        self.enqueue_dialog(PendingKind::Editor, "editor", fields)
    }

    pub fn notify(&mut self, message: &str, notify_type: Option<&str>) -> Value {
        let kind = notify_type.unwrap_or("info");
        self.notifications
            .push((kind.to_string(), message.to_string()));
        let mut fields = json!({ "message": message });
        if let Some(notify_type) = notify_type {
            fields["notifyType"] = json!(notify_type);
        }
        Self::request("notify", fields)
    }

    pub fn set_status(&mut self, key: &str, text: Option<&str>) -> Value {
        match text {
            Some(text) => {
                self.statuses.insert(key.to_string(), text.to_string());
            }
            None => {
                self.statuses.remove(key);
            }
        }
        Self::request(
            "setStatus",
            json!({
                "statusKey": key,
                "statusText": text,
            }),
        )
    }

    pub fn set_widget(
        &mut self,
        key: &str,
        lines: Option<&[String]>,
        placement: Option<&str>,
    ) -> Value {
        match lines {
            Some(lines) => {
                self.widgets.insert(
                    key.to_string(),
                    Widget {
                        lines: lines.to_vec(),
                        placement: placement.unwrap_or("aboveEditor").to_string(),
                    },
                );
            }
            None => {
                self.widgets.remove(key);
            }
        }
        Self::request(
            "setWidget",
            json!({
                "widgetKey": key,
                "widgetLines": lines,
                "widgetPlacement": placement.unwrap_or("aboveEditor"),
            }),
        )
    }

    pub fn set_title(&mut self, title: &str) -> Value {
        self.title = Some(title.to_string());
        Self::request("setTitle", json!({ "title": title }))
    }

    pub fn set_editor_text(&mut self, text: &str) -> Value {
        self.editor_text = text.to_string();
        Self::request("set_editor_text", json!({ "text": text }))
    }

    pub fn paste_to_editor(&mut self, text: &str) -> Value {
        // TypeScript RPC `pasteToEditor` falls back to `setEditorText`.
        self.set_editor_text(text)
    }

    /// Clear statuses/widgets/editor like TypeScript `resetExtensionUI`.
    pub fn reset(&mut self) -> Vec<Value> {
        let status_keys: Vec<String> = self.statuses.keys().cloned().collect();
        let widget_keys: Vec<String> = self.widgets.keys().cloned().collect();
        let mut events = Vec::new();
        for key in status_keys {
            events.push(self.set_status(&key, None));
        }
        for key in widget_keys {
            events.push(self.set_widget(&key, None, None));
        }
        self.notifications.clear();
        self.pending.clear();
        events.push(self.set_editor_text(""));
        events
    }

    pub fn dispatch(&mut self, method: &str, fields: &Value) -> Option<Value> {
        if !METHODS.contains(&method) {
            return None;
        }
        let timeout = fields.get("timeout").and_then(|v| v.as_u64());
        match method {
            "select" => {
                let title = fields.get("title").and_then(|v| v.as_str()).unwrap_or("");
                let options = fields
                    .get("options")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                Some(self.select(title, &options, timeout))
            }
            "confirm" => Some(self.confirm(
                fields.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                fields.get("message").and_then(|v| v.as_str()).unwrap_or(""),
                timeout,
            )),
            "input" => Some(self.input(
                fields.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                fields.get("placeholder").and_then(|v| v.as_str()),
                timeout,
            )),
            "editor" => Some(self.editor(
                fields.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                fields.get("prefill").and_then(|v| v.as_str()),
            )),
            "notify" => Some(self.notify(
                fields.get("message").and_then(|v| v.as_str()).unwrap_or(""),
                fields.get("notifyType").and_then(|v| v.as_str()),
            )),
            "setStatus" => Some(
                self.set_status(
                    fields
                        .get("statusKey")
                        .and_then(|v| v.as_str())
                        .unwrap_or(""),
                    fields.get("statusText").and_then(|v| v.as_str()),
                ),
            ),
            "setWidget" => {
                let lines = fields
                    .get("widgetLines")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect::<Vec<_>>()
                    });
                Some(
                    self.set_widget(
                        fields
                            .get("widgetKey")
                            .and_then(|v| v.as_str())
                            .unwrap_or(""),
                        lines.as_deref(),
                        fields.get("widgetPlacement").and_then(|v| v.as_str()),
                    ),
                )
            }
            "setTitle" => {
                Some(self.set_title(fields.get("title").and_then(|v| v.as_str()).unwrap_or("")))
            }
            "set_editor_text" => Some(
                self.paste_to_editor(fields.get("text").and_then(|v| v.as_str()).unwrap_or("")),
            ),
            _ => None,
        }
    }

    pub fn apply_calls(&mut self, calls: &[Value]) -> Vec<Value> {
        calls
            .iter()
            .filter_map(|call| {
                let method = call.get("method").and_then(|v| v.as_str())?;
                self.dispatch(method, call)
            })
            .collect()
    }

    pub fn parse_response(command: &Value) -> UiResponse {
        if command.get("cancelled").and_then(|v| v.as_bool()) == Some(true) {
            return UiResponse::Cancelled;
        }
        if let Some(confirmed) = command.get("confirmed").and_then(|v| v.as_bool()) {
            return UiResponse::Confirmed(confirmed);
        }
        if let Some(value) = command.get("value").and_then(|v| v.as_str()) {
            return UiResponse::Value(value.to_string());
        }
        UiResponse::Cancelled
    }

    /// Correlate `extension_ui_response` the way TypeScript `pendingExtensionRequests` does.
    pub fn apply_response(&mut self, command: &Value) -> Value {
        let id = command.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let parsed = Self::parse_response(command);
        let kind = self.pending.remove(id);
        match (kind, parsed) {
            (Some(PendingKind::Confirm), UiResponse::Cancelled) => {
                json!({"confirmed": false, "cancelled": true})
            }
            (Some(PendingKind::Confirm), UiResponse::Confirmed(confirmed)) => {
                json!({"confirmed": confirmed})
            }
            (Some(PendingKind::Confirm), UiResponse::Value(value)) => {
                json!({"confirmed": value.eq_ignore_ascii_case("yes") || value == "true"})
            }
            (
                Some(PendingKind::Select | PendingKind::Input | PendingKind::Editor),
                UiResponse::Cancelled,
            ) => json!({"cancelled": true}),
            (
                Some(PendingKind::Select | PendingKind::Input | PendingKind::Editor),
                UiResponse::Value(value),
            ) => json!({"value": value}),
            (_, UiResponse::Value(value)) => json!({"value": value}),
            (_, UiResponse::Confirmed(confirmed)) => json!({"confirmed": confirmed}),
            (_, UiResponse::Cancelled) => json!({"cancelled": true}),
        }
    }

    pub fn status_line(&self) -> String {
        let mut keys: Vec<_> = self.statuses.keys().cloned().collect();
        keys.sort();
        keys.into_iter()
            .filter_map(|k| self.statuses.get(&k).map(|v| format!("{k}:{v}")))
            .collect::<Vec<_>>()
            .join("  ")
    }

    pub fn widget_lines(&self, placement: &str) -> Vec<String> {
        let mut keys: Vec<_> = self.widgets.keys().cloned().collect();
        keys.sort();
        keys.into_iter()
            .filter_map(|k| self.widgets.get(&k).cloned())
            .filter(|w| w.placement == placement)
            .flat_map(|w| w.lines)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_shapes_match_rpc_types() {
        let mut host = ExtensionUiHost::default();
        let select = host.select("Pick", &["A".into(), "B".into()], Some(1000));
        assert_eq!(select["type"], "extension_ui_request");
        assert_eq!(select["method"], "select");
        assert_eq!(select["title"], "Pick");
        assert_eq!(select["options"][0], "A");
        assert_eq!(select["timeout"], 1000);
        assert!(host.pending.contains_key(select["id"].as_str().unwrap()));

        let confirm = host.confirm("Sure?", "All messages will be lost.", None);
        assert_eq!(confirm["method"], "confirm");
        assert_eq!(confirm["message"], "All messages will be lost.");

        let input = host.input("Name", Some("pi"), None);
        assert_eq!(input["method"], "input");
        assert_eq!(input["placeholder"], "pi");

        let editor = host.editor("Edit", Some("draft"));
        assert_eq!(editor["method"], "editor");
        assert_eq!(editor["prefill"], "draft");

        let notify = host.notify("hello", Some("warning"));
        assert_eq!(notify["method"], "notify");
        assert_eq!(notify["notifyType"], "warning");

        let status = host.set_status("rpc-demo", Some("Turns: 1"));
        assert_eq!(status["method"], "setStatus");
        assert_eq!(status["statusKey"], "rpc-demo");
        assert_eq!(status["statusText"], "Turns: 1");

        let widget = host.set_widget(
            "rpc-demo",
            Some(&["--- RPC Extension UI Demo ---".into()]),
            Some("aboveEditor"),
        );
        assert_eq!(widget["method"], "setWidget");
        assert_eq!(widget["widgetKey"], "rpc-demo");
        assert_eq!(widget["widgetPlacement"], "aboveEditor");

        let title = host.set_title("pi RPC Demo");
        assert_eq!(title["method"], "setTitle");
        assert_eq!(title["title"], "pi RPC Demo");

        let editor_text = host.set_editor_text("typed");
        assert_eq!(editor_text["method"], "set_editor_text");
        assert_eq!(editor_text["text"], "typed");
        assert_eq!(host.paste_to_editor("pasted")["method"], "set_editor_text");

        for method in METHODS {
            assert!([
                "select",
                "confirm",
                "input",
                "editor",
                "notify",
                "setStatus",
                "setWidget",
                "setTitle",
                "set_editor_text"
            ]
            .contains(method));
        }
    }

    #[test]
    fn correlates_responses_like_typescript() {
        let mut host = ExtensionUiHost::default();
        let select = host.select("Pick", &["Allow".into(), "Block".into()], None);
        let id = select["id"].as_str().unwrap().to_string();
        let data = host.apply_response(&json!({
            "type": "extension_ui_response",
            "id": id,
            "value": "Allow"
        }));
        assert_eq!(data["value"], "Allow");
        assert!(host.pending.is_empty());

        let confirm = host.confirm("Clear?", "lost", None);
        let id = confirm["id"].as_str().unwrap().to_string();
        let data = host.apply_response(&json!({
            "type": "extension_ui_response",
            "id": id,
            "cancelled": true
        }));
        assert_eq!(data["confirmed"], false);
        assert_eq!(data["cancelled"], true);
    }
}
