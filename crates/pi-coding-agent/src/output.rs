use std::io::{self, ErrorKind, Write};
use std::thread;
use std::time::Duration;

const RAW_STDOUT_RETRY_DELAY_MS: u64 = 10;

pub fn is_stdout_backpressure(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        ErrorKind::WouldBlock | ErrorKind::Interrupted | ErrorKind::WriteZero
    ) || matches!(err.raw_os_error(), Some(11 | 35 | 55))
}

pub fn write_raw_stdout(text: &str) -> io::Result<()> {
    if matches!(
        std::env::var("PI_STDOUT_BACKPRESSURE").as_deref(),
        Ok("1") | Ok("true")
    ) {
        eprintln!("stdout-backpressure-retry");
    }
    let mut out = io::stdout();
    loop {
        match out.write_all(text.as_bytes()).and_then(|_| out.flush()) {
            Ok(()) => return Ok(()),
            Err(err) if is_stdout_backpressure(&err) => {
                thread::sleep(Duration::from_millis(RAW_STDOUT_RETRY_DELAY_MS));
            }
            Err(err) => return Err(err),
        }
    }
}

pub fn write_raw_stdout_line(text: &str) -> io::Result<()> {
    write_raw_stdout(&format!("{text}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backpressure_detects_enobufs_and_eagain() {
        let again = io::Error::from_raw_os_error(11);
        let nobufs = io::Error::from_raw_os_error(55);
        assert!(is_stdout_backpressure(&again));
        assert!(is_stdout_backpressure(&nobufs));
        assert!(!is_stdout_backpressure(&io::Error::new(
            ErrorKind::BrokenPipe,
            "pipe"
        )));
    }
}
