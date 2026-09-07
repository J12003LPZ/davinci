//! Local voice host supervision; both interactive loops share this controller.
use crate::voice_models::{self, Config};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use davinci_tui::davinci::{app, model::Model};
use davinci_voice::{
    protocol::{self, Command, Event, VoiceError},
    state::{Controller, Effect, Phase},
};
use std::{
    process::{Command as ProcessCommand, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

enum Message {
    Event(Event),
    Reaped,
}
enum SetupEvent {
    Progress(u64),
    Done(Result<(), String>),
}
struct SetupJob {
    rx: Receiver<SetupEvent>,
    cancel: Arc<AtomicBool>,
}
impl Drop for SetupJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
struct Worker {
    tx: SyncSender<Command>,
    rx: Receiver<Message>,
    kill: Arc<AtomicBool>,
}

impl Worker {
    fn spawn() -> Result<Self, VoiceError> {
        let path = voice_models::helper_path().map_err(|_| VoiceError::WorkerCrashed)?;
        if !path.is_file() {
            return Err(VoiceError::WorkerCrashed);
        }
        let (tx, commands) = mpsc::sync_channel(protocol::CHANNEL_CAPACITY);
        let (events, rx) = mpsc::sync_channel(protocol::CHANNEL_CAPACITY);
        let kill = Arc::new(AtomicBool::new(false));
        let stop = kill.clone();
        thread::spawn(move || {
            let mut command = ProcessCommand::new(path);
            command
                .env_clear()
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            for name in [
                "SystemRoot",
                "WINDIR",
                "TEMP",
                "TMP",
                "HOME",
                "USER",
                "XDG_RUNTIME_DIR",
                "DBUS_SESSION_BUS_ADDRESS",
                "PULSE_SERVER",
                "DISPLAY",
            ] {
                if let Some(value) = std::env::var_os(name) {
                    command.env(name, value);
                }
            }
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x08000000);
            }
            let Ok(mut child) = command.spawn() else {
                let _ = events.try_send(Message::Reaped);
                return;
            };
            let mut input = child.stdin.take().expect("piped stdin");
            let mut output = child.stdout.take().expect("piped stdout");
            let write_stop = stop.clone();
            let writer = thread::spawn(move || {
                while !write_stop.load(Ordering::Acquire) {
                    match commands.recv_timeout(Duration::from_millis(25)) {
                        Ok(command) => {
                            if protocol::write_frame(&mut input, &command).is_err() {
                                break;
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(_) => break,
                    }
                }
            });
            let read_events = events.clone();
            let read_stop = stop.clone();
            let reader = thread::spawn(move || {
                while let Ok(Some(event)) = protocol::read_frame::<_, Event>(&mut output) {
                    if read_events.try_send(Message::Event(event)).is_err() {
                        read_stop.store(true, Ordering::Release);
                        break;
                    }
                }
                read_stop.store(true, Ordering::Release);
            });
            loop {
                if stop.load(Ordering::Acquire) {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Err(_) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                    Ok(None) => {}
                }
                thread::sleep(Duration::from_millis(10));
            }
            stop.store(true, Ordering::Release);
            let _ = writer.join();
            let _ = reader.join();
            let _ = events.send(Message::Reaped);
        });
        Ok(Self { tx, rx, kill })
    }
    fn send(&self, command: Command) -> Result<(), VoiceError> {
        self.tx
            .try_send(command)
            .map_err(|_| VoiceError::WorkerCrashed)
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.kill.store(true, Ordering::Release);
    }
}

pub struct VoiceInput {
    controller: Controller,
    config: Config,
    worker: Option<Worker>,
    clock: Instant,
    handshake: bool,
    notice: String,
    setup_open: bool,
    setup_choice: usize,
    import_path: Option<String>,
    setup_job: Option<SetupJob>,
}

impl VoiceInput {
    pub fn new(model: &mut Model) -> Self {
        let (config, notice) = match Config::load() {
            Ok(config) => (config, String::new()),
            Err(error) => (
                Config {
                    enabled: false,
                    ..Config::default()
                },
                error,
            ),
        };
        let (keys, conflict) = model.keybindings.with_voice(config.enabled);
        model.keybindings = keys;
        let this = Self {
            controller: Controller::default(),
            config,
            worker: None,
            clock: Instant::now(),
            handshake: false,
            notice: conflict.unwrap_or(notice),
            setup_open: false,
            setup_choice: 0,
            import_path: None,
            setup_job: None,
        };
        this.project(model);
        this
    }
    fn now(&self) -> u64 {
        self.clock.elapsed().as_millis().min(u64::MAX as u128) as u64
    }

    fn send(&mut self, command: Command) {
        if self
            .worker
            .as_ref()
            .ok_or(VoiceError::WorkerCrashed)
            .and_then(|w| w.send(command))
            .is_err()
        {
            if let Some(worker) = &self.worker {
                worker.kill.store(true, Ordering::Release);
            }
            self.controller.error = Some(VoiceError::WorkerCrashed);
            let _ = self.controller.cancel(self.now());
        }
    }

    fn effect(&mut self, effect: Effect, model: &mut Model) {
        match effect {
            Effect::Prepare(_) => {
                let ready = voice_models::model_path(&self.config.model).is_ok_and(|p| p.is_file());
                if !ready {
                    self.controller.failed(VoiceError::ModelMissing);
                    self.setup_open = true;
                    self.notice.clear();
                    return;
                }
                if self.handshake && self.worker.is_some() {
                    if let (Some(id), Ok(path)) = (
                        self.controller.identity,
                        voice_models::model_path(&self.config.model),
                    ) {
                        self.send(Command::Prepare {
                            id,
                            model: self.config.model.clone(),
                            path,
                        });
                    }
                    return;
                }
                match Worker::spawn() {
                    Ok(worker) => {
                        self.worker = Some(worker);
                        self.handshake = false;
                        self.send(Command::Hello {
                            version: protocol::VERSION,
                            build: protocol::BUILD.into(),
                        });
                    }
                    Err(error) => self.controller.failed(error),
                }
            }
            Effect::Start(id) => self.send(Command::Start {
                id,
                language: self.config.language.clone(),
                device: self.config.input_device.clone(),
            }),
            Effect::Stop(session) => self.send(Command::Stop { session }),
            Effect::Cancel(session) => self.send(Command::Cancel { session }),
            Effect::Kill => {
                if let Some(worker) = &self.worker {
                    worker.kill.store(true, Ordering::Release);
                }
            }
            Effect::Insert { epoch, text } => match model.insert_dictation(&text, epoch) {
                Ok(true) => {
                    self.notice = "Inserted / review, then Enter to send / Ctrl+- undo".into()
                }
                Ok(false) => self.notice = "No speech inserted".into(),
                Err(_) => self.notice = VoiceError::TextTooLong.to_string(),
            },
            Effect::OpenSetup => {
                self.notice =
                    "Run davinci voice model install base or model import base <path>".into()
            }
        }
    }

    pub fn cancel(&mut self, model: &mut Model) {
        if self.setup_open {
            self.setup_job.take();
            self.setup_open = false;
            self.import_path = None;
        }
        if let Some(effect) = self.controller.cancel(self.now()) {
            self.effect(effect, model);
        }
        self.project(model);
    }

    pub fn toggle(&mut self, model: &mut Model) {
        if !self.config.enabled
            || !model.voice_eligible()
            || (self.worker.is_some() && !self.handshake && !self.controller.phase.active())
        {
            return;
        }
        model.dismiss_suggestions();
        if let Some(effect) = self.controller.toggle(self.now(), model.composer_epoch) {
            self.effect(effect, model);
        }
        self.project(model);
    }

    /// Reserved actions precede raw extensions, completion and busy queueing.
    pub fn key(&mut self, model: &mut Model, key: KeyEvent) -> bool {
        if self.setup_open {
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                self.cancel(model);
                return false;
            }
            self.setup_key(key);
            self.project(model);
            return true;
        }
        let data = davinci_tui::key_event_bytes(&key);
        if self.config.enabled
            && model.voice_eligible()
            && data
                .as_ref()
                .is_some_and(|d| model.keybindings.matches(d, "davinci.voice.toggle"))
        {
            if key.kind == KeyEventKind::Press {
                self.toggle(model);
            }
            return true;
        }
        if self.controller.phase.active() {
            if key.code == KeyCode::Esc {
                self.cancel(model);
                return true;
            }
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                self.cancel(model);
                return false;
            }
        }
        if self.controller.blocks_send(self.now()) && app::voice_send_key(model, &key) {
            self.notice = "Finish or cancel voice before sending".into();
            self.project(model);
            return true;
        }
        false
    }

    pub fn tick(&mut self, model: &mut Model, input_pending: bool) {
        let now = self.now();
        if self.setup_open
            && (model.overlay.is_some()
                || model.screen != davinci_tui::davinci::model::Screen::Agent
                || model.codex_open())
        {
            self.cancel(model);
        }
        for _ in 0..16 {
            let Some(event) = self
                .setup_job
                .as_ref()
                .and_then(|job| job.rx.try_recv().ok())
            else {
                break;
            };
            match event {
                SetupEvent::Progress(bytes) => {
                    self.notice = format!("Received {bytes} bytes / Esc cancels setup")
                }
                SetupEvent::Done(result) => {
                    self.setup_job.take();
                    self.import_path = None;
                    match result {
                        Ok(()) => {
                            self.setup_open = false;
                            self.controller.error = None;
                            self.controller.closed();
                            self.notice = "Ready / Ctrl+T to listen. Nothing recorded.".into();
                        }
                        Err(error) => self.notice = error,
                    }
                }
            }
        }
        if self.controller.phase.active()
            && (!model.voice_eligible()
                || model.height < 4
                || self
                    .controller
                    .identity
                    .is_some_and(|id| id.epoch != model.composer_epoch))
        {
            self.cancel(model);
        }
        let inserted = self.controller.phase == Phase::Inserted;
        if let Some(effect) = self.controller.tick(now) {
            self.effect(effect, model);
        }
        if inserted && self.controller.phase == Phase::Idle {
            self.notice.clear();
        }
        if !input_pending {
            for _ in 0..protocol::CHANNEL_CAPACITY {
                let Some(message) = self.worker.as_ref().and_then(|w| w.rx.try_recv().ok()) else {
                    break;
                };
                match message {
                    Message::Reaped => {
                        self.worker.take();
                        self.handshake = false;
                        if self.controller.phase == Phase::Cancelling {
                            self.controller.closed();
                        } else if self.controller.phase.active() {
                            self.controller.failed(VoiceError::WorkerCrashed);
                        }
                    }
                    Message::Event(Event::Hello { version, build }) => {
                        if version != protocol::VERSION
                            || build != protocol::BUILD
                            || self.handshake
                        {
                            self.controller.error = Some(VoiceError::ProtocolMismatch);
                            self.cancel(model);
                        } else if let Some(id) = self.controller.identity {
                            self.handshake = true;
                            if let Ok(path) = voice_models::model_path(&self.config.model) {
                                self.send(Command::Prepare {
                                    id,
                                    model: self.config.model.clone(),
                                    path,
                                });
                            }
                        }
                    }
                    Message::Event(event) if self.handshake => self.event(event, model, now),
                    Message::Event(_) => {
                        self.controller.error = Some(VoiceError::ProtocolMismatch);
                        self.cancel(model);
                    }
                }
            }
        }
        self.project(model);
    }

    fn event(&mut self, event: Event, model: &mut Model, now: u64) {
        let id = match &event {
            Event::Ready { id }
            | Event::CaptureStarted { id }
            | Event::CaptureStopped { id }
            | Event::Transcribing { id }
            | Event::Completed { id, .. }
            | Event::Cancelled { id }
            | Event::Failed { id, .. } => *id,
            Event::Hello { .. } => return,
        };
        if self.controller.identity != Some(id) {
            return;
        }
        match event {
            Event::Ready { .. } => {
                if let Some(effect) = self.controller.ready(id.session, model.composer_epoch, now) {
                    self.effect(effect, model);
                }
            }
            Event::CaptureStarted { .. } => self.controller.capture_started(id.session, now),
            Event::CaptureStopped { .. } => self.controller.capture_stopped(id.session, now),
            Event::Completed { text, .. } => {
                if model.voice_eligible() && id.epoch == model.composer_epoch {
                    if let Some(effect) = self.controller.complete(id.session, id.epoch, text, now)
                    {
                        self.effect(effect, model);
                    }
                }
            }
            Event::Cancelled { .. } => {
                self.controller.closed();
                self.send(Command::Shutdown);
                self.handshake = false;
            }
            Event::Failed { error, .. } => {
                self.controller.failed(error);
                self.send(Command::Shutdown);
                self.handshake = false;
            }
            _ => {}
        }
    }

    pub fn drawn(&mut self) {
        self.controller.drawn();
    }

    pub fn terminal_handoff(&mut self, model: &mut Model) {
        self.cancel(model);
        if let Some(worker) = &self.worker {
            worker.kill.store(true, Ordering::Release);
        }
    }

    pub fn polling(&self) -> bool {
        self.controller.phase.active()
            || self.controller.phase == Phase::Inserted
            || self.setup_job.is_some()
    }

    pub fn open_setup(&mut self, model: &mut Model) {
        self.cancel(model);
        if self.config.enabled {
            self.setup_open = true;
            self.notice.clear();
        } else {
            self.notice = "Voice is disabled in global settings".into();
        }
        self.project(model);
    }

    pub fn paste(&mut self, model: &mut Model, text: &str) -> bool {
        if !self.setup_open {
            return false;
        }
        if let Some(path) = &mut self.import_path {
            if path.len() + text.len() <= 4096 && !text.chars().any(char::is_control) {
                path.push_str(text);
            }
        }
        self.project(model);
        true
    }

    fn setup_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        if key.code == KeyCode::Esc {
            if let Some(job) = &self.setup_job {
                job.cancel.store(true, Ordering::Release);
                self.notice = "Cancelling setup...".into();
            } else {
                self.setup_open = false;
                self.import_path = None;
            }
            return;
        }
        if self.setup_job.is_some() {
            return;
        }
        if let Some(path) = &mut self.import_path {
            match key.code {
                KeyCode::Backspace => {
                    path.pop();
                }
                KeyCode::Char(ch) if !ch.is_control() && path.len() < 4096 => path.push(ch),
                KeyCode::Enter if !path.trim().is_empty() => {
                    let source = std::path::PathBuf::from(path.trim().trim_matches('"'));
                    self.start_setup(Some(source));
                }
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Up => self.setup_choice = (self.setup_choice + 2) % 3,
            KeyCode::Down | KeyCode::Tab => self.setup_choice = (self.setup_choice + 1) % 3,
            KeyCode::Enter => match self.setup_choice {
                0 => self.start_setup(None),
                1 => self.import_path = Some(String::new()),
                _ => self.setup_open = false,
            },
            _ => {}
        }
    }

    fn start_setup(&mut self, source: Option<std::path::PathBuf>) {
        let id = self.config.model.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (tx, rx) = mpsc::sync_channel(16);
        thread::spawn(move || {
            let result = voice_models::setup(&id, source.as_deref(), &worker_cancel, &|bytes| {
                let _ = tx.try_send(SetupEvent::Progress(bytes));
            });
            let _ = tx.send(SetupEvent::Done(result));
        });
        self.setup_job = Some(SetupJob { rx, cancel });
        self.notice = "Starting model setup / Esc cancels".into();
    }

    pub fn project(&self, model: &mut Model) {
        let shortcut = model
            .keybindings
            .keys_for("davinci.voice.toggle")
            .first()
            .map(String::as_str)
            .unwrap_or("click");
        model.voice.setup = self.setup_open;
        model.voice.setup_rows = if self.setup_open {
            let bytes = davinci_voice::catalog::find(&self.config.model).map_or(0, |m| m.bytes);
            let mut rows = vec![
                "Local voice setup".into(),
                format!("Whisper {} / MIT / {bytes} bytes", self.config.model),
                "Recognition runs locally after setup. Downloads need Internet.".into(),
                format!(
                    "Destination: {}",
                    voice_models::model_path(&self.config.model)
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                ),
                "Installing never records or sends. Your draft is preserved.".into(),
                String::new(),
            ];
            if let Some(path) = &self.import_path {
                rows.push(format!("Approved model path: {path}"));
                rows.push("Enter import / Esc cancel".into());
            } else {
                for (index, label) in [
                    "Download model (consent to download)",
                    "Import existing approved model",
                    "Cancel",
                ]
                .iter()
                .enumerate()
                {
                    rows.push(format!(
                        "{} {label}",
                        if self.setup_choice == index { ">" } else { " " }
                    ));
                }
            }
            rows.push(self.notice.clone());
            if model.height < 11 {
                let choices = rows.split_off(6);
                rows = vec!["Voice setup / Esc cancel".into()];
                rows.extend(choices);
            }
            rows
        } else {
            Vec::new()
        };
        model.voice.enabled = self.config.enabled;
        model.voice.mouse = self.config.mouse;
        model.voice.active = self.controller.phase.active();
        model.voice.blocks_send = self.controller.blocks_send(self.now());
        model.voice.notice = self
            .controller
            .error
            .map(|e| e.to_string())
            .unwrap_or_else(|| self.notice.clone());
        model.voice.label = match self.controller.phase {
            Phase::Listening => format!(
                "REC {:02}:{:02} / {shortcut} stop",
                self.controller.elapsed_seconds(self.now()) / 60,
                self.controller.elapsed_seconds(self.now()) % 60
            ),
            Phase::Preparing => "Preparing / Esc cancel".into(),
            Phase::Stopping => "Stopping microphone".into(),
            Phase::Transcribing => "Transcribing / Esc cancel".into(),
            Phase::Cancelling => "Cancelling voice".into(),
            _ => format!("mic {shortcut}"),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fake() -> (VoiceInput, Model, SyncSender<Message>, Receiver<Command>) {
        let mut model = davinci_tui::davinci::boot(&[], 100, 30);
        model.screen = davinci_tui::davinci::model::Screen::Agent;
        model.overlay = None;
        let (tx, commands) = mpsc::sync_channel(16);
        let (events, rx) = mpsc::sync_channel(16);
        let mut controller = Controller::default();
        controller.toggle(0, model.composer_epoch);
        controller.capture_started(1, 1);
        controller.capture_stopped(1, 2);
        let voice = VoiceInput {
            controller,
            config: Config::default(),
            worker: Some(Worker {
                tx,
                rx,
                kill: Arc::new(AtomicBool::new(false)),
            }),
            clock: Instant::now(),
            handshake: true,
            notice: String::new(),
            setup_open: false,
            setup_choice: 0,
            import_path: None,
            setup_job: None,
        };
        voice.project(&mut model);
        (voice, model, events, commands)
    }
    #[test]
    fn final_waits_for_pending_input_and_never_sends_or_queues() {
        for running in [false, true] {
            let (mut voice, mut model, events, _commands) = fake();
            model.running = running;
            model.composer.push_str("typed");
            let before = model.transcript.len();
            let id = voice.controller.identity.unwrap();
            events
                .send(Message::Event(Event::Completed {
                    id,
                    text: "spoken".into(),
                }))
                .unwrap();
            voice.tick(&mut model, true);
            assert_eq!(model.composer.to_string(), "typed");
            let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
            assert!(voice.key(&mut model, enter));
            voice.tick(&mut model, false);
            assert_eq!(model.composer.to_string(), "typed spoken");
            assert!(voice.key(&mut model, enter));
            assert_eq!(model.transcript.len(), before);
            assert!(model.queued.is_empty());
            assert_eq!(model.running, running);
            events
                .send(Message::Event(Event::Completed {
                    id,
                    text: "duplicate".into(),
                }))
                .unwrap();
            voice.tick(&mut model, false);
            assert_eq!(model.composer.to_string(), "typed spoken");
        }
    }
    #[test]
    fn cancellation_and_modal_keep_live_edits() {
        let (mut voice, mut model, events, _commands) = fake();
        let id = voice.controller.identity.unwrap();
        model.composer.push_str("keep");
        assert!(voice.key(&mut model, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        model.composer.push_str(" edits");
        events
            .send(Message::Event(Event::Completed {
                id,
                text: "late".into(),
            }))
            .unwrap();
        voice.tick(&mut model, false);
        assert_eq!(model.composer.to_string(), "keep edits");
        assert_eq!(voice.controller.phase, Phase::Cancelling);
    }

    #[test]
    fn setup_cancel_preserves_draft_and_ctrl_c_falls_through() {
        let (mut voice, mut model, _events, _commands) = fake();
        voice.controller.closed();
        model.composer.push_str("draft");
        voice.open_setup(&mut model);
        assert!(model.voice.setup);
        voice.tick(&mut model, false);
        assert!(model.voice.setup);
        assert!(!voice.key(
            &mut model,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
        ));
        assert!(!model.voice.setup);
        assert_eq!(model.composer.to_string(), "draft");
        assert!(!voice.controller.phase.active());
    }

    #[test]
    fn epoch_change_discards_completion_and_blocks_autocomplete_enter() {
        let (mut voice, mut model, events, _commands) = fake();
        let id = voice.controller.identity.unwrap();
        model.replace_composer("/model");
        events
            .send(Message::Event(Event::Completed {
                id,
                text: "late".into(),
            }))
            .unwrap();
        voice.tick(&mut model, false);
        assert!(voice.key(
            &mut model,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
        ));
        assert_eq!(model.composer.to_string(), "/model");
        assert!(model.overlay.is_none());
        assert_eq!(voice.controller.phase, Phase::Cancelling);
    }
}
