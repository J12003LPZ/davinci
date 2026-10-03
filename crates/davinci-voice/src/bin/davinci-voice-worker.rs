//! Native worker entry; no agent/provider runtime is linked into this process.
use davinci_voice::{
    audio,
    capture::{self, Capture},
    engine::Engine,
    protocol::{self, Command, Event, VoiceError},
    state::Identity,
};
use std::{
    io,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

fn emit(tx: &SyncSender<Event>, event: Event) -> Result<(), VoiceError> {
    tx.try_send(event).map_err(|_| VoiceError::WorkerCrashed)
}

fn owner(
    rx: Receiver<Command>,
    tx: SyncSender<Event>,
    cancel: Arc<AtomicBool>,
    active: Arc<AtomicU64>,
) -> Result<(), VoiceError> {
    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Command::Hello {
            version: protocol::VERSION,
            build,
        }) if build == protocol::BUILD => {}
        _ => return Err(VoiceError::ProtocolMismatch),
    }
    emit(
        &tx,
        Event::Hello {
            version: protocol::VERSION,
            build: protocol::BUILD.into(),
        },
    )?;
    let mut engine: Option<Engine> = None;
    let mut loaded_model = None;
    let mut identity: Option<Identity> = None;
    let mut capture: Option<Capture> = None;
    let mut language = String::from("auto");
    let mut warm_until = Instant::now() + Duration::from_secs(300);
    loop {
        let wait = if capture.is_some() {
            Duration::from_millis(10)
        } else {
            warm_until.saturating_duration_since(Instant::now())
        };
        let command = match rx.recv_timeout(wait) {
            Ok(command) => Some(command),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(_) => break,
        };
        let mut stop = false;
        match command {
            Some(Command::Shutdown) => break,
            Some(Command::Prepare { id, model, path }) if identity.is_none() => {
                cancel.store(false, Ordering::Release);
                active.store(id.session, Ordering::Release);
                identity = Some(id);
                if loaded_model.as_ref() == Some(&(model.clone(), path.clone())) && engine.is_some()
                {
                    emit(&tx, Event::Ready { id })?;
                    continue;
                }
                match Engine::load(&model, &path) {
                    Ok(loaded) => {
                        loaded_model = Some((model, path));
                        engine = Some(loaded);
                        emit(&tx, Event::Ready { id })?;
                    }
                    Err(error) => {
                        emit(&tx, Event::Failed { id, error })?;
                        identity = None;
                    }
                }
            }
            Some(Command::Start {
                id,
                language: requested,
                device,
            }) if identity == Some(id) && capture.is_none() && !cancel.load(Ordering::Acquire) => {
                language = requested;
                match Capture::start(device.as_deref()) {
                    Ok(stream) => {
                        capture = Some(stream);
                        emit(&tx, Event::CaptureStarted { id })?;
                    }
                    Err(error) => {
                        emit(&tx, Event::Failed { id, error })?;
                        identity = None;
                    }
                }
            }
            Some(Command::Stop { session }) if identity.is_some_and(|id| id.session == session) => {
                stop = capture.is_some();
            }
            Some(Command::Cancel { session })
                if identity.is_some_and(|id| id.session == session) =>
            {
                cancel.store(true, Ordering::Release);
            }
            Some(_) | None => {}
        }
        if cancel.load(Ordering::Acquire) {
            capture.take();
            if let Some(id) = identity.take() {
                emit(&tx, Event::Cancelled { id })?;
            }
            cancel.store(false, Ordering::Release);
            warm_until = Instant::now() + Duration::from_secs(300);
        }
        if let Some(stream) = capture.as_mut() {
            match stream.drain() {
                Ok(limit) => stop |= limit,
                Err(error) => {
                    capture.take();
                    if let Some(id) = identity.take() {
                        emit(&tx, Event::Failed { id, error })?;
                    }
                }
            }
        }
        if stop {
            if let (Some(stream), Some(id)) = (capture.take(), identity) {
                let pcm = stream.stop();
                emit(&tx, Event::CaptureStopped { id })?;
                emit(&tx, Event::Transcribing { id })?;
                let result = pcm.and_then(|pcm| {
                    if audio::no_speech(&pcm) {
                        return Err(VoiceError::NoSpeech);
                    }
                    engine
                        .as_mut()
                        .ok_or(VoiceError::ModelMissing)?
                        .decode(&pcm, &language, &cancel)
                });
                let event = if cancel.load(Ordering::Acquire) {
                    Event::Cancelled { id }
                } else {
                    match result {
                        Ok(text) if !text.is_empty() => Event::Completed { id, text },
                        Ok(_) => Event::Failed {
                            id,
                            error: VoiceError::NoSpeech,
                        },
                        Err(error) => Event::Failed { id, error },
                    }
                };
                emit(&tx, event)?;
                identity = None;
                warm_until = Instant::now() + Duration::from_secs(300);
            }
        }
        if identity.is_none() && Instant::now() >= warm_until {
            break;
        }
    }
    Ok(())
}

fn run() -> Result<(), VoiceError> {
    let cancel = Arc::new(AtomicBool::new(false));
    let active = Arc::new(AtomicU64::new(0));
    let parent_lost = Arc::new(AtomicBool::new(false));
    let (commands, rx) = mpsc::sync_channel(protocol::CHANNEL_CAPACITY);
    let (tx, events) = mpsc::sync_channel(protocol::CHANNEL_CAPACITY);
    let reader_cancel = cancel.clone();
    let reader_lost = parent_lost.clone();
    let reader_active = active.clone();
    thread::spawn(move || {
        let mut input = io::stdin().lock();
        while let Ok(Some(command)) = protocol::read_frame::<_, Command>(&mut input) {
            if matches!(command, Command::Shutdown)
                || matches!(command,Command::Cancel {session} if session == reader_active.load(Ordering::Acquire))
            {
                reader_cancel.store(true, Ordering::Release);
            }
            if commands.try_send(command).is_err() {
                break;
            }
        }
        reader_cancel.store(true, Ordering::Release);
        reader_lost.store(true, Ordering::Release);
    });
    let writer_lost = parent_lost.clone();
    let writer = thread::spawn(move || {
        let mut output = io::stdout().lock();
        while let Ok(event) = events.recv() {
            if protocol::write_frame(&mut output, &event).is_err() {
                writer_lost.store(true, Ordering::Release);
                break;
            }
        }
    });
    thread::spawn(move || {
        while !parent_lost.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(250));
        }
        thread::sleep(Duration::from_millis(750));
        std::process::exit(1);
    });
    let result = owner(rx, tx, cancel, active);
    let _ = writer.join();
    result
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--devices") {
        match capture::devices() {
            Ok(devices) => {
                for device in devices {
                    println!("{device}");
                }
                return;
            }
            Err(_) => std::process::exit(1),
        }
    }
    if run().is_err() {
        std::process::exit(1);
    }
}
