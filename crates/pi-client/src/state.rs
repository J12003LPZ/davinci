//! In-memory client state matching TypeScript `packages/client/src/state.ts`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use pi_protocol::{CommandResult, ServerEvent, ServerSnapshot, SessionSnapshot};

pub type Unsubscribe = Box<dyn FnOnce()>;

#[derive(Clone, Default)]
pub struct ClientState {
    inner: Rc<RefCell<Inner>>,
}

#[derive(Default)]
struct Inner {
    snapshot: Option<ServerSnapshot>,
    session_snapshots: HashMap<String, SessionSnapshot>,
    attached_session_ids: HashSet<String>,
    snapshot_listeners: HashMap<u64, Rc<dyn Fn(&ServerSnapshot)>>,
    event_listeners: HashMap<u64, Rc<dyn Fn(&ServerEvent)>>,
    session_snapshot_listeners: HashMap<String, HashMap<u64, Rc<dyn Fn(&SessionSnapshot)>>>,
    session_event_listeners: HashMap<String, HashMap<u64, Rc<dyn Fn(&ServerEvent)>>>,
    next_id: u64,
}

impl ClientState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> Option<ServerSnapshot> {
        self.inner.borrow().snapshot.clone()
    }

    pub fn reset(&self) {
        let mut inner = self.inner.borrow_mut();
        inner.snapshot = None;
        inner.session_snapshots.clear();
        inner.attached_session_ids.clear();
    }

    pub fn clear_attachments(&self) {
        self.inner.borrow_mut().attached_session_ids.clear();
    }

    pub fn dispose(&self) {
        self.reset();
        let mut inner = self.inner.borrow_mut();
        inner.snapshot_listeners.clear();
        inner.event_listeners.clear();
        inner.session_snapshot_listeners.clear();
        inner.session_event_listeners.clear();
    }

    pub fn get_session_snapshot(&self, session_id: &str) -> Option<SessionSnapshot> {
        self.inner
            .borrow()
            .session_snapshots
            .get(session_id)
            .cloned()
    }

    pub fn is_session_attached(&self, session_id: &str) -> bool {
        self.inner
            .borrow()
            .attached_session_ids
            .contains(session_id)
    }

    pub fn forget_session_snapshot(&self, session_id: &str) -> Option<SessionSnapshot> {
        self.inner.borrow_mut().session_snapshots.remove(session_id)
    }

    pub fn restore_session_snapshot(&self, snapshot: SessionSnapshot) {
        let mut inner = self.inner.borrow_mut();
        inner
            .session_snapshots
            .entry(snapshot.id.clone())
            .or_insert(snapshot);
    }

    pub fn subscribe(&self, listener: impl Fn(&ServerSnapshot) + 'static) -> Unsubscribe {
        let listener = Rc::new(listener);
        let id = {
            let mut inner = self.inner.borrow_mut();
            inner.next_id += 1;
            let id = inner.next_id;
            inner.snapshot_listeners.insert(id, listener);
            id
        };
        let inner = self.inner.clone();
        Box::new(move || {
            inner.borrow_mut().snapshot_listeners.remove(&id);
        })
    }

    pub fn on_event(&self, listener: impl Fn(&ServerEvent) + 'static) -> Unsubscribe {
        let listener = Rc::new(listener);
        let id = {
            let mut inner = self.inner.borrow_mut();
            inner.next_id += 1;
            let id = inner.next_id;
            inner.event_listeners.insert(id, listener);
            id
        };
        let inner = self.inner.clone();
        Box::new(move || {
            inner.borrow_mut().event_listeners.remove(&id);
        })
    }

    pub fn subscribe_session(
        &self,
        session_id: impl Into<String>,
        listener: impl Fn(&SessionSnapshot) + 'static,
    ) -> Unsubscribe {
        let session_id = session_id.into();
        let listener = Rc::new(listener);
        let id = {
            let mut inner = self.inner.borrow_mut();
            inner.next_id += 1;
            let id = inner.next_id;
            inner
                .session_snapshot_listeners
                .entry(session_id.clone())
                .or_default()
                .insert(id, listener);
            id
        };
        let inner = self.inner.clone();
        Box::new(move || {
            let mut inner = inner.borrow_mut();
            if let Some(listeners) = inner.session_snapshot_listeners.get_mut(&session_id) {
                listeners.remove(&id);
                if listeners.is_empty() {
                    inner.session_snapshot_listeners.remove(&session_id);
                }
            }
        })
    }

    pub fn on_session_event(
        &self,
        session_id: impl Into<String>,
        listener: impl Fn(&ServerEvent) + 'static,
    ) -> Unsubscribe {
        let session_id = session_id.into();
        let listener = Rc::new(listener);
        let id = {
            let mut inner = self.inner.borrow_mut();
            inner.next_id += 1;
            let id = inner.next_id;
            inner
                .session_event_listeners
                .entry(session_id.clone())
                .or_default()
                .insert(id, listener);
            id
        };
        let inner = self.inner.clone();
        Box::new(move || {
            let mut inner = inner.borrow_mut();
            if let Some(listeners) = inner.session_event_listeners.get_mut(&session_id) {
                listeners.remove(&id);
                if listeners.is_empty() {
                    inner.session_event_listeners.remove(&session_id);
                }
            }
        })
    }

    pub fn apply_result(&self, result: &CommandResult) {
        match result {
            CommandResult::List { .. } => {}
            CommandResult::Detach { session_id } => {
                let mut inner = self.inner.borrow_mut();
                inner.attached_session_ids.remove(session_id);
                if let Some(snapshot) = inner.session_snapshots.get(session_id).cloned() {
                    drop(inner);
                    let mut next = snapshot;
                    next.attached = false;
                    self.apply_session_snapshot(next, true);
                }
            }
            CommandResult::Create { session }
            | CommandResult::Attach { session }
            | CommandResult::Prompt { session }
            | CommandResult::Steer { session }
            | CommandResult::Abort { session }
            | CommandResult::SetModel { session }
            | CommandResult::SetThinking { session } => {
                self.apply_session_snapshot(session.clone(), false);
            }
        }
    }

    pub fn apply_event(&self, event: &ServerEvent) {
        match event {
            ServerEvent::ServerSnapshot { snapshot } => {
                self.apply_server_snapshot(snapshot.clone());
            }
            ServerEvent::SessionSnapshot { snapshot } => {
                self.apply_session_snapshot(snapshot.clone(), false);
            }
            ServerEvent::SessionRemoved { session_id } => {
                let mut inner = self.inner.borrow_mut();
                inner.session_snapshots.remove(session_id);
                inner.attached_session_ids.remove(session_id);
            }
            ServerEvent::SessionProgress { .. } => {}
        }
        let listeners: Vec<Rc<dyn Fn(&ServerEvent)>> = {
            let inner = self.inner.borrow();
            inner.event_listeners.values().cloned().collect()
        };
        for listener in listeners {
            listener(event);
        }
        if let Some(session_id) = event_session_id(event) {
            let listeners: Vec<Rc<dyn Fn(&ServerEvent)>> = {
                let inner = self.inner.borrow();
                inner
                    .session_event_listeners
                    .get(session_id)
                    .map(|set| set.values().cloned().collect())
                    .unwrap_or_default()
            };
            for listener in listeners {
                listener(event);
            }
        }
    }

    pub fn apply_server_snapshot(&self, snapshot: ServerSnapshot) {
        let listeners = {
            let mut inner = self.inner.borrow_mut();
            if inner
                .snapshot
                .as_ref()
                .is_some_and(|current| snapshot.revision < current.revision)
            {
                return;
            }
            inner.snapshot = Some(snapshot.clone());
            inner
                .snapshot_listeners
                .values()
                .cloned()
                .collect::<Vec<_>>()
        };
        for listener in listeners {
            listener(&snapshot);
        }
    }

    fn apply_session_snapshot(&self, snapshot: SessionSnapshot, force: bool) {
        let listeners = {
            let mut inner = self.inner.borrow_mut();
            if !force {
                if let Some(current) = inner.session_snapshots.get(&snapshot.id) {
                    if snapshot.revision < current.revision {
                        return;
                    }
                }
            }
            if snapshot.attached {
                inner.attached_session_ids.insert(snapshot.id.clone());
            } else {
                inner.attached_session_ids.remove(&snapshot.id);
            }
            let id = snapshot.id.clone();
            inner.session_snapshots.insert(id.clone(), snapshot.clone());
            inner
                .session_snapshot_listeners
                .get(&id)
                .map(|set| {
                    set.values()
                        .cloned()
                        .collect::<Vec<Rc<dyn Fn(&SessionSnapshot)>>>()
                })
                .unwrap_or_default()
        };
        for listener in listeners {
            listener(&snapshot);
        }
    }
}

fn event_session_id(event: &ServerEvent) -> Option<&str> {
    match event {
        ServerEvent::SessionSnapshot { snapshot } => Some(snapshot.id.as_str()),
        ServerEvent::SessionProgress { session_id, .. }
        | ServerEvent::SessionRemoved { session_id } => Some(session_id.as_str()),
        ServerEvent::ServerSnapshot { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pi_protocol::{ModelRef, SessionPhase, ThinkingLevel};

    fn session(id: &str, revision: u64, attached: bool) -> SessionSnapshot {
        SessionSnapshot {
            id: id.into(),
            name: Some("demo".into()),
            cwd: "/tmp".into(),
            created_at: 1,
            updated_at: 1,
            phase: SessionPhase::Idle,
            model: ModelRef {
                provider: "google".into(),
                id: "gemini".into(),
            },
            thinking_level: ThinkingLevel::Off,
            attached,
            locked: false,
            revision,
            transcript: Vec::new(),
            queued_steer: Vec::new(),
            queued_steer_count: 0,
        }
    }

    #[test]
    fn apply_result_and_detach_mark_unattached() {
        let state = ClientState::new();
        let events = Rc::new(RefCell::new(Vec::new()));
        let events_clone = events.clone();
        let _unsub = state.subscribe_session("s1", move |snapshot| {
            events_clone.borrow_mut().push(snapshot.attached);
        });
        state.apply_result(&CommandResult::Create {
            session: session("s1", 1, true),
        });
        assert!(state.is_session_attached("s1"));
        state.apply_result(&CommandResult::Detach {
            session_id: "s1".into(),
        });
        assert!(!state.is_session_attached("s1"));
        assert_eq!(
            state.get_session_snapshot("s1").map(|item| item.attached),
            Some(false)
        );
        assert_eq!(*events.borrow(), vec![true, false]);
    }

    #[test]
    fn revision_checks_and_event_subscribe() {
        let state = ClientState::new();
        state.apply_event(&ServerEvent::SessionSnapshot {
            snapshot: session("s1", 5, true),
        });
        state.apply_event(&ServerEvent::SessionSnapshot {
            snapshot: session("s1", 3, false),
        });
        assert_eq!(
            state.get_session_snapshot("s1").map(|item| item.revision),
            Some(5)
        );
        assert!(state.is_session_attached("s1"));
        let seen = Rc::new(RefCell::new(0u32));
        let seen_clone = seen.clone();
        let unsub = state.on_event(move |_| {
            *seen_clone.borrow_mut() += 1;
        });
        state.apply_event(&ServerEvent::SessionRemoved {
            session_id: "s1".into(),
        });
        assert!(state.get_session_snapshot("s1").is_none());
        assert!(!state.is_session_attached("s1"));
        assert_eq!(*seen.borrow(), 1);
        unsub();
        state.apply_event(&ServerEvent::SessionRemoved {
            session_id: "s1".into(),
        });
        assert_eq!(*seen.borrow(), 1);
    }

    #[test]
    fn server_snapshot_ignores_older_revision() {
        let state = ClientState::new();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let seen_clone = seen.clone();
        let _unsub = state.subscribe(move |snapshot| {
            seen_clone.borrow_mut().push(snapshot.revision);
        });
        state.apply_server_snapshot(ServerSnapshot {
            server_id: "srv".into(),
            protocol_version: 1,
            revision: 4,
            sessions: Vec::new(),
            models: Vec::new(),
        });
        state.apply_server_snapshot(ServerSnapshot {
            server_id: "srv".into(),
            protocol_version: 1,
            revision: 2,
            sessions: Vec::new(),
            models: Vec::new(),
        });
        assert_eq!(state.snapshot().map(|item| item.revision), Some(4));
        assert_eq!(*seen.borrow(), vec![4]);
    }
}
