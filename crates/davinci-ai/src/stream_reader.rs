//! Provider-body reader, independent of decoding and foreground cancellation.
use std::io::{self, BufRead, BufReader, Read};
use std::sync::mpsc::{self, Receiver};

pub(crate) const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
const READ_AHEAD_LINES: usize = 8;

pub(crate) fn response_lines(reader: impl Read + Send + 'static) -> Receiver<io::Result<String>> {
    let (sender, receiver) = mpsc::sync_channel(READ_AHEAD_LINES);
    std::thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut frame_bytes = 0;
        loop {
            let mut bytes = Vec::new();
            let remaining = MAX_FRAME_BYTES - frame_bytes;
            // A limit on read_until also bounds a peer that never sends a
            // newline; checking String length after read_line would be too late.
            let read = reader
                .by_ref()
                .take(remaining as u64 + 1)
                .read_until(b'\n', &mut bytes);
            match read {
                Ok(0) => break,
                Ok(count) => {
                    if count > remaining {
                        let _ = sender.send(Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "provider frame exceeds 16 MiB",
                        )));
                        break;
                    }
                    let line = match String::from_utf8(bytes) {
                        Ok(line) => line,
                        Err(error) => {
                            let _ =
                                sender.send(Err(io::Error::new(io::ErrorKind::InvalidData, error)));
                            break;
                        }
                    };
                    frame_bytes = if line.trim_end_matches(['\n', '\r']).is_empty() {
                        0
                    } else {
                        frame_bytes + count
                    };
                    if sender.send(Ok(line)).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = sender.send(Err(error));
                    break;
                }
            }
        }
    });
    receiver
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::Sender;
    use std::time::Duration;

    struct ObservedReader {
        lines: usize,
        full: Sender<()>,
        excess: Sender<()>,
        dropped: Sender<()>,
    }
    impl Read for ObservedReader {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            self.lines += 1;
            if self.lines == READ_AHEAD_LINES + 1 {
                let _ = self.full.send(());
            }
            if self.lines == READ_AHEAD_LINES + 2 {
                let _ = self.excess.send(());
            }
            if self.lines > 1000 {
                return Ok(0);
            }
            bytes[0] = b'\n';
            Ok(1)
        }
    }
    impl Drop for ObservedReader {
        fn drop(&mut self) {
            let _ = self.dropped.send(());
        }
    }

    #[test]
    fn audit_regression_reader_backpressure_and_cancellation() {
        let (full, ready) = mpsc::channel();
        let (excess, overflow) = mpsc::channel();
        let (dropped, finished) = mpsc::channel();
        let receiver = response_lines(ObservedReader {
            lines: 0,
            full,
            excess,
            dropped,
        });
        ready.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(
            overflow.recv_timeout(Duration::from_millis(50)).is_err(),
            "reader ran ahead of the consumer"
        );
        drop(receiver);
        finished.recv_timeout(Duration::from_secs(2)).unwrap();
    }

    #[test]
    fn audit_regression_oversized_lines_and_multiline_frames_are_rejected() {
        let line = vec![b'x'; MAX_FRAME_BYTES + 1];
        let result = response_lines(io::Cursor::new(line)).recv().unwrap();
        assert!(result.is_err(), "oversized provider line was accepted");
        let error = result.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);

        let fragment = format!("data: {}\n", "x".repeat(1024));
        let body = fragment.repeat(MAX_FRAME_BYTES / fragment.len() + 1);
        let receiver = response_lines(io::Cursor::new(body));
        let mut rejected = false;
        for line in receiver {
            if let Err(error) = line {
                assert_eq!(error.kind(), io::ErrorKind::InvalidData);
                rejected = true;
            }
        }
        assert!(rejected, "unterminated multi-line frame was accepted");
    }

    #[test]
    fn reader_preserves_lines_and_resets_the_frame_budget() {
        let data = format!("data: {}\n\n", "x".repeat(MAX_FRAME_BYTES / 2));
        let expected = data.repeat(3);
        let actual = response_lines(io::Cursor::new(expected.clone()))
            .into_iter()
            .collect::<io::Result<Vec<_>>>()
            .unwrap()
            .concat();
        assert_eq!(actual, expected);
    }

    #[test]
    fn exact_limit_includes_the_blank_frame_terminator() {
        for ending in ["\n\n", "\r\n\r\n"] {
            let body = format!("{}{}", "x".repeat(MAX_FRAME_BYTES - ending.len()), ending);
            assert_eq!(body.len(), MAX_FRAME_BYTES);
            let received = response_lines(io::Cursor::new(body.clone()))
                .into_iter()
                .collect::<io::Result<Vec<_>>>()
                .unwrap()
                .concat();
            assert_eq!(received, body);

            let oversized = format!("x{body}");
            let result = response_lines(io::Cursor::new(oversized))
                .into_iter()
                .collect::<io::Result<Vec<_>>>();
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
        }
    }

    #[test]
    fn invalid_utf8_is_reported_without_forwarding_partial_data() {
        let receiver = response_lines(io::Cursor::new(b"data: \xff\n\n".to_vec()));
        assert_eq!(
            receiver.recv().unwrap().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(receiver.recv().is_err());
    }
}
