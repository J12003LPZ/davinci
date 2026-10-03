//! Interactive role choices are drafts until the operator selects Start graph.
use super::*;
use crate::native_extensions::graph::{GraphCommand, Role};
use std::collections::BTreeMap;

const ROLES: [Role; 7] = [
    Role::Classifier,
    Role::Researcher,
    Role::TestAnalyzer,
    Role::Historian,
    Role::Planner,
    Role::Writer,
    Role::Reviewer,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setup {
    args: String,
    models: BTreeMap<Role, String>,
    available: Vec<String>,
    selecting: Option<Role>,
    query: String,
}

pub fn is_launch(args: &str) -> bool {
    use crate::native_extensions::graph as render;
    matches!(render::parse_advanced_graph_command(args), Ok(None))
        && matches!(
            render::parse_graph_command(args),
            Ok(GraphCommand::Goal(_) | GraphCommand::RunSaved { .. })
        )
}

impl Setup {
    fn filtered(&self) -> Vec<&str> {
        let query = self.query.to_lowercase();
        self.available
            .iter()
            .filter(|name| name.to_lowercase().contains(&query))
            .map(String::as_str)
            .collect()
    }

    pub fn ask(&self, agent: &Agent) -> Ask {
        let inherited = format!("{}/{}", agent.provider, agent.model_id);
        let (title, note, items) = if let Some(role) = self.selecting {
            let mut items = vec![PickerItem::new("Use session model", &inherited)];
            items.extend(
                self.filtered()
                    .into_iter()
                    .map(|name| PickerItem::new(name, "Use for this role")),
            );
            (
                format!("Graph model: {}", role.as_str()),
                format!("Type to filter: {} · Esc returns to roles", self.query),
                items,
            )
        } else {
            let mut items: Vec<_> = ROLES
                .iter()
                .map(|role| {
                    let selected = self.models.get(role).unwrap_or(&inherited);
                    PickerItem::new(
                        &format!("{}: {selected}", role.as_str()),
                        "Enter to choose a model",
                    )
                })
                .collect();
            items.push(PickerItem::new(
                "Start graph",
                "Launch with these role models",
            ));
            items.push(PickerItem::new(
                "Use session model for all roles",
                &inherited,
            ));
            items.push(PickerItem::new("Cancel", "No workers will start"));
            (
                "Graph setup".into(),
                "Choose models before workers start. Choices are kept for this session.".into(),
                items,
            )
        };
        Ask {
            title,
            name: String::new(),
            key: "/graph setup".into(),
            note,
            items,
            ..Default::default()
        }
    }
}

fn show(shell: &mut Shell<'_>, setup: Setup) {
    shell.model.running = false;
    shell.model.ask = setup.ask(shell.agent);
    *shell.pending = Some(Question::GraphSetup(setup));
    open_ask_overlay(shell.model);
}

pub fn open(shell: &mut Shell<'_>, args: &str) {
    let models = {
        let host = shell.host.lock().unwrap_or_else(|err| err.into_inner());
        crate::apply_graph_session_context(shell.parsed, shell.agent, &host);
        let native = host.native.lock().unwrap_or_else(|err| err.into_inner());
        native.graph.role_models()
    };
    let mut available: Vec<_> = crate::load_model_runtime(shell.parsed)
        .available
        .iter()
        .map(|entry| format!("{}/{}", entry.provider, entry.id))
        .collect();
    available.sort();
    available.dedup();
    show(
        shell,
        Setup {
            args: args.into(),
            models,
            available,
            selecting: None,
            query: String::new(),
        },
    );
}

pub fn choose(shell: &mut Shell<'_>, mut setup: Setup, index: usize) -> Next {
    if let Some(role) = setup.selecting {
        if index == 0 {
            setup.models.remove(&role);
        } else if let Some(name) = setup.filtered().get(index - 1) {
            setup.models.insert(role, (*name).to_string());
        } else {
            show(shell, setup);
            return Next::Go;
        }
        setup.selecting = None;
        setup.query.clear();
    } else if let Some(role) = ROLES.get(index) {
        setup.selecting = Some(*role);
    } else if index == ROLES.len() {
        // Recheck the live catalog at launch; an unavailable configured model
        // must not silently fall back to a different model.
        let available: std::collections::BTreeSet<_> = crate::load_model_runtime(shell.parsed)
            .available
            .iter()
            .map(|entry| format!("{}/{}", entry.provider, entry.id))
            .collect();
        if let Some(name) = setup
            .models
            .values()
            .find(|name| !available.contains(*name))
        {
            let notice = format!("Model unavailable: {name}. Choose another model or use /login.");
            show(shell, setup);
            shell.note(&notice);
            return Next::Go;
        }
        {
            let host = shell.host.lock().unwrap_or_else(|err| err.into_inner());
            let mut native = host.native.lock().unwrap_or_else(|err| err.into_inner());
            native.graph.set_role_models(setup.models);
        }
        shell.model.close();
        return run_extension_command_inner(shell, &format!("/graph {}", setup.args), false)
            .unwrap_or(Next::Go);
    } else if index == ROLES.len() + 1 {
        setup.models.clear();
    } else {
        shell.model.close();
        return Next::Go;
    }
    show(shell, setup);
    Next::Go
}

pub fn key(
    model: &mut Model,
    pending: &mut Option<Question>,
    agent: &Agent,
    key: crossterm::event::KeyEvent,
) -> bool {
    use crossterm::event::{KeyCode, KeyModifiers};
    if model.overlay != Some(Overlay::Ask) {
        return false;
    }
    let Some(Question::GraphSetup(setup)) = pending.as_mut() else {
        return false;
    };
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        *pending = None;
        model.close();
        return true;
    }
    if key.code == KeyCode::Esc {
        if setup.selecting.take().is_some() {
            setup.query.clear();
            model.ask = setup.ask(agent);
            open_ask_overlay(model);
        } else {
            *pending = None;
            model.close();
        }
        return true;
    }
    if setup.selecting.is_none() {
        return false;
    }
    match key.code {
        KeyCode::Char(ch)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            setup.query.push(ch)
        }
        KeyCode::Backspace => {
            setup.query.pop();
        }
        _ => return false,
    }
    model.ask = setup.ask(agent);
    open_ask_overlay(model);
    if !setup.query.is_empty() && model.ask.items.len() > 1 {
        model.ask_index = 1;
    }
    true
}

pub fn paste(model: &mut Model, pending: &mut Option<Question>, agent: &Agent, text: &str) -> bool {
    if model.overlay != Some(Overlay::Ask) {
        return false;
    }
    let Some(Question::GraphSetup(setup)) = pending.as_mut() else {
        return false;
    };
    if setup.selecting.is_some() {
        setup
            .query
            .extend(text.chars().filter(|ch| !ch.is_control()).take(256));
        model.ask = setup.ask(agent);
        open_ask_overlay(model);
        if !setup.query.is_empty() && model.ask.items.len() > 1 {
            model.ask_index = 1;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_new_graphs_open_setup() {
        for args in [
            "fix the parser",
            "--dry-run improve tests",
            "run security-audit --dry-run",
        ] {
            assert!(is_launch(args), "{args}");
        }
        for args in [
            "",
            "save example",
            "diff",
            "export example",
            "explain writer",
        ] {
            assert!(!is_launch(args), "{args}");
        }
    }

    #[test]
    fn role_model_filter_preserves_exact_provider_ids() {
        let setup = Setup {
            args: "task".into(),
            models: BTreeMap::new(),
            available: vec!["provider/gpt-luna".into(), "provider/gpt-sol".into()],
            selecting: Some(Role::Researcher),
            query: "LUNA".into(),
        };
        assert_eq!(setup.filtered(), vec!["provider/gpt-luna"]);
        assert_eq!(ROLES.len(), 7);
    }

    #[test]
    fn search_and_escape_keep_the_main_model_and_never_start_workers() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut agent = Agent::new("fixture");
        agent.provider = "provider".into();
        agent.model_id = "chat".into();
        let mut model = Model::new(
            davinci_tui::davinci::theme::Theme::da_vinci(
                davinci_tui::davinci::theme::ColorDepth::TrueColor,
                false,
            ),
            100,
            30,
            false,
        );
        let setup = Setup {
            args: "task".into(),
            models: BTreeMap::new(),
            available: vec!["provider/luna".into(), "provider/sol".into()],
            selecting: Some(Role::Researcher),
            query: String::new(),
        };
        model.ask = setup.ask(&agent);
        open_ask_overlay(&mut model);
        let mut pending = Some(Question::GraphSetup(setup));
        assert!(key(
            &mut model,
            &mut pending,
            &agent,
            KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE)
        ));
        assert_eq!(model.ask_index, 1);
        assert_eq!(model.ask.items[1].label, "provider/luna");
        assert!(paste(&mut model, &mut pending, &agent, "una\r\n"));
        assert_eq!(model.ask.items.len(), 2);
        assert_eq!(model.ask.items[1].label, "provider/luna");
        assert_eq!(model.ask_index, 1);
        assert!(!model.running);
        assert!(key(
            &mut model,
            &mut pending,
            &agent,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
        ));
        assert_eq!(model.ask.title, "Graph setup");
        assert!(key(
            &mut model,
            &mut pending,
            &agent,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
        ));
        assert!(pending.is_none());
        assert!(model.overlay.is_none());
        assert!(!model.running);
        assert_eq!(agent.model_id, "chat");
    }
}
