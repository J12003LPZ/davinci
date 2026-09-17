use super::{
    platform::Ownership,
    wire::{self, Event, Request, MAX_INPUT, POLL},
    ProcessConfig,
};
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};

type Message = (Event, Option<mpsc::SyncSender<()>>);

pub(super) fn run() -> ! {
    let ownership = match Ownership::enter() {
        Ok(value) => value,
        Err(_) => std::process::exit(1),
    };
    let _ = run_owned();
    ownership.terminate()
}

fn run_owned() -> std::io::Result<()> {
    let mut output = std::io::stdout();
    output.write_all(wire::MAGIC)?;
    wire::write(
        &mut output,
        &Event::Hello {
            pid: std::process::id(),
        },
    )?;
    let mut input = std::io::stdin();
    let Request::Configure(config) = wire::read(&mut input)? else {
        return Ok(());
    };
    let mut child = spawn(config)?;
    wire::write(&mut output, &Event::Started { pid: child.id() })?;

    let stopped = Arc::new(AtomicBool::new(false));
    let (events, event_rx) = mpsc::sync_channel::<Message>(64);
    let writer_stop = stopped.clone();
    thread::spawn(move || {
        let mut output = std::io::stdout();
        while let Ok((event, ack)) = event_rx.recv() {
            if wire::write(&mut output, &event).is_err() {
                writer_stop.store(true, Ordering::SeqCst);
                break;
            }
            if let Some(ack) = ack {
                let _ = ack.try_send(());
            }
        }
    });
    let readers: Vec<_> = [
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    ]
    .into_iter()
    .flatten()
    .map(|mut pipe| {
        let events = events.clone();
        thread::spawn(move || {
            let mut bytes = [0; 8192];
            loop {
                let count = match pipe.read(&mut bytes) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                if events
                    .send((
                        Event::Output {
                            bytes: bytes[..count].to_vec(),
                        },
                        None,
                    ))
                    .is_err()
                {
                    break;
                }
            }
        })
    })
    .collect();
    let (writes, write_rx) = mpsc::sync_channel::<(u64, Vec<u8>)>(1);
    let write_events = events.clone();
    let mut stdin = child.stdin.take().expect("piped stdin");
    thread::spawn(move || {
        while let Ok((id, bytes)) = write_rx.recv() {
            let failed = stdin.write_all(&bytes).and_then(|_| stdin.flush()).is_err();
            let count = if failed { 0 } else { bytes.len() };
            if write_events
                .send((Event::Written { id, count, failed }, None))
                .is_err()
            {
                break;
            }
        }
    });
    let input_stop = stopped.clone();
    let input_events = events.clone();
    thread::spawn(move || {
        while let Ok(Request::Write { id, bytes }) = wire::read(&mut input) {
            if bytes.len() > MAX_INPUT {
                break;
            }
            if writes.try_send((id, bytes)).is_err()
                && input_events
                    .send((
                        Event::Written {
                            id,
                            count: 0,
                            failed: true,
                        },
                        None,
                    ))
                    .is_err()
            {
                break;
            }
        }
        // This pipe is the parent lifeline. EOF also covers SIGKILL/TerminateProcess.
        input_stop.store(true, Ordering::SeqCst);
    });

    while !stopped.load(Ordering::SeqCst) {
        if let Some(status) = child.try_wait()? {
            // A grandchild may retain the pipes. Bound tail draining before
            // terminating the group instead of waiting for those pipes forever.
            let deadline = Instant::now() + Duration::from_millis(100);
            while readers.iter().any(|reader| !reader.is_finished()) && Instant::now() < deadline {
                thread::sleep(POLL);
            }
            flush_exit(
                &events,
                status.code(),
                readers.iter().all(|reader| reader.is_finished()),
            );
            return Ok(());
        }
        thread::sleep(POLL);
    }
    Ok(())
}

fn spawn(config: ProcessConfig) -> std::io::Result<std::process::Child> {
    let mut command = Command::new(config.executable);
    command
        .args(config.argv)
        .current_dir(config.cwd)
        .env_clear()
        .envs(config.environment)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    command.spawn()
}

fn flush_exit(events: &mpsc::SyncSender<Message>, code: Option<i32>, output_complete: bool) {
    let (ack, ack_rx) = mpsc::sync_channel(1);
    let mut message = (
        Event::Exit {
            code,
            output_complete,
        },
        Some(ack),
    );
    let deadline = Instant::now() + Duration::from_millis(200);
    loop {
        match events.try_send(message) {
            Ok(()) => {
                let _ = ack_rx.recv_timeout(deadline.saturating_duration_since(Instant::now()));
                break;
            }
            Err(mpsc::TrySendError::Full(returned)) if Instant::now() < deadline => {
                message = returned;
                thread::sleep(POLL);
            }
            Err(_) => break,
        }
    }
}
