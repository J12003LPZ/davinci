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
    .enumerate()
    .filter_map(|(index, pipe)| pipe.map(|pipe| (index == 1, pipe)))
    .map(|(stderr, pipe)| {
        let events = events.clone();
        thread::spawn(move || forward_output(pipe, &events, stderr))
    })
    .collect();
    let (writes, write_rx) = mpsc::sync_channel::<(u64, Option<Vec<u8>>)>(1);
    let write_events = events.clone();
    let mut stdin = child.stdin.take();
    thread::spawn(move || {
        while let Ok((id, bytes)) = write_rx.recv() {
            let result = match bytes {
                Some(bytes) => stdin
                    .as_mut()
                    .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::BrokenPipe))
                    .and_then(|stdin| stdin.write_all(&bytes).and_then(|_| stdin.flush()))
                    .map(|_| bytes.len()),
                None => {
                    drop(stdin.take());
                    Ok(0)
                }
            };
            let failed = result.is_err();
            let count = result.unwrap_or(0);
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
        while let Ok(request) = wire::read(&mut input) {
            let (id, bytes) = match request {
                Request::Write { id, bytes } if bytes.len() <= MAX_INPUT => (id, Some(bytes)),
                Request::CloseStdin { id } => (id, None),
                _ => break,
            };
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
            flush_exit(&events, status.code(), readers_complete(readers));
            return Ok(());
        }
        thread::sleep(POLL);
    }
    Ok(())
}

fn forward_output(
    mut pipe: impl Read,
    events: &mpsc::SyncSender<Message>,
    stderr: bool,
) -> std::io::Result<()> {
    let mut bytes = [0; 8192];
    loop {
        let count = match pipe.read(&mut bytes) {
            Ok(0) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
            Ok(n) => n,
        };
        if events
            .send((
                Event::Output {
                    bytes: bytes[..count].to_vec(),
                    stderr,
                },
                None,
            ))
            .is_err()
        {
            return Err(std::io::ErrorKind::BrokenPipe.into());
        }
    }
}

fn readers_complete(readers: Vec<thread::JoinHandle<std::io::Result<()>>>) -> bool {
    let mut complete = true;
    for reader in readers {
        // Never join a descendant-held pipe that exceeded the drain deadline.
        complete &= reader.is_finished() && matches!(reader.join(), Ok(Ok(())));
    }
    complete
}

fn spawn(config: ProcessConfig) -> std::io::Result<std::process::Child> {
    #[cfg(windows)]
    let cwd = {
        // Node and other runtimes cannot resolve relative files from a verbatim
        // current directory. Preserve the authorized location: simplify only
        // when both spellings resolve to the same canonical directory.
        let ordinary = crate::permission::strip_verbatim_prefix(&config.cwd);
        if ordinary.canonicalize()? == config.cwd.canonicalize()? {
            ordinary
        } else {
            config.cwd
        }
    };
    #[cfg(not(windows))]
    let cwd = config.cwd;
    let mut command = Command::new(config.executable);
    command
        .args(config.argv)
        .current_dir(cwd)
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

#[cfg(test)]
mod tests {
    use super::*;

    struct InterruptedThenData(bool);
    impl Read for InterruptedThenData {
        fn read(&mut self, _bytes: &mut [u8]) -> std::io::Result<usize> {
            if !self.0 {
                self.0 = true;
                return Err(std::io::ErrorKind::Interrupted.into());
            }
            Err(std::io::ErrorKind::Other.into())
        }
    }

    #[test]
    fn supervisor_capture_does_not_report_read_failure_as_eof() {
        let (events, _receiver) = mpsc::sync_channel(2);
        let result = forward_output(InterruptedThenData(false), &events, false);
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::Other);
    }

    #[test]
    fn supervisor_capture_requires_output_delivery() {
        let (events, receiver) = mpsc::sync_channel(2);
        drop(receiver);
        assert!(forward_output(&b"output"[..], &events, false).is_err());
    }

    #[test]
    fn supervisor_capture_completion_requires_successful_readers() {
        for failed in [false, true] {
            let reader = thread::spawn(move || {
                if failed {
                    Err(std::io::ErrorKind::Other.into())
                } else {
                    Ok(())
                }
            });
            while !reader.is_finished() {
                thread::yield_now();
            }
            assert_eq!(readers_complete(vec![reader]), !failed);
        }
        let reader = thread::spawn(|| -> std::io::Result<()> { panic!("reader fixture") });
        while !reader.is_finished() {
            thread::yield_now();
        }
        assert!(!readers_complete(vec![reader]));
        let (release, wait) = mpsc::sync_channel::<()>(1);
        let reader = thread::spawn(move || {
            let _ = wait.recv();
            Ok(())
        });
        assert!(!readers_complete(vec![reader]));
        drop(release);
    }
}
