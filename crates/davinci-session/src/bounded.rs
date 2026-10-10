//! Line reads with a byte ceiling, for session files whose size nobody
//! promised. `BufRead::read_line` buffers a newline-free record whole, so one
//! damaged file could make a listing allocate its entire size.

use std::io::{self, BufRead};

/// The largest header line a listing reads. Real headers are a few hundred
/// bytes; metadata keeps them far under this.
pub(crate) const MAX_HEADER_LINE_BYTES: usize = 256 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Line {
    /// `buf` holds the line, without its `\n`.
    Complete,
    /// The line is longer than the cap. `buf` is empty and the reader stops
    /// inside the line; call [`skip_line`] to move past it.
    TooLong,
    Eof,
}

/// Read one line of at most `max` bytes into `buf` (cleared first).
pub(crate) fn read_line_capped(
    reader: &mut impl BufRead,
    max: usize,
    buf: &mut Vec<u8>,
) -> io::Result<Line> {
    buf.clear();
    let mut read_any = false;
    loop {
        let available = match reader.fill_buf() {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        };
        if available.is_empty() {
            return Ok(if read_any { Line::Complete } else { Line::Eof });
        }
        read_any = true;
        let newline = available.iter().position(|byte| *byte == b'\n');
        let take = newline.unwrap_or(available.len());
        if buf.len() + take > max {
            buf.clear();
            return Ok(Line::TooLong);
        }
        buf.extend_from_slice(&available[..take]);
        reader.consume(newline.map_or(take, |index| index + 1));
        if newline.is_some() {
            return Ok(Line::Complete);
        }
    }
}

/// Consume the rest of the current line, through its `\n`, without keeping it.
pub(crate) fn skip_line(reader: &mut impl BufRead) -> io::Result<()> {
    loop {
        let available = match reader.fill_buf() {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        };
        if available.is_empty() {
            return Ok(());
        }
        match available.iter().position(|byte| *byte == b'\n') {
            Some(index) => {
                reader.consume(index + 1);
                return Ok(());
            }
            None => {
                let len = available.len();
                reader.consume(len);
            }
        }
    }
}

/// The first line of `path`, if it is at most [`MAX_HEADER_LINE_BYTES`] and
/// UTF-8. Anything else is not a header a listing should trust.
pub(crate) fn read_header_line(path: &std::path::Path) -> Option<String> {
    let mut reader = io::BufReader::new(std::fs::File::open(path).ok()?);
    let mut buf = Vec::new();
    match read_line_capped(&mut reader, MAX_HEADER_LINE_BYTES, &mut buf).ok()? {
        Line::Complete => String::from_utf8(buf).ok(),
        Line::TooLong | Line::Eof => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capped_lines_stop_without_buffering_and_skip_resumes_after() {
        // A tiny BufReader capacity forces the multi-chunk paths.
        let data = format!("short\n{}\nafter", "x".repeat(10_000));
        let mut reader = io::BufReader::with_capacity(7, data.as_bytes());
        let mut buf = Vec::new();
        assert_eq!(
            read_line_capped(&mut reader, 100, &mut buf).unwrap(),
            Line::Complete
        );
        assert_eq!(buf, b"short");
        assert_eq!(
            read_line_capped(&mut reader, 100, &mut buf).unwrap(),
            Line::TooLong
        );
        assert!(buf.is_empty());
        // Growth may round up, but never toward the 10 000-byte line.
        assert!(buf.capacity() <= 256, "{} bytes reserved", buf.capacity());
        skip_line(&mut reader).unwrap();
        assert_eq!(
            read_line_capped(&mut reader, 100, &mut buf).unwrap(),
            Line::Complete
        );
        assert_eq!(buf, b"after");
        assert_eq!(
            read_line_capped(&mut reader, 100, &mut buf).unwrap(),
            Line::Eof
        );
    }

    #[test]
    fn a_line_exactly_at_the_cap_is_complete() {
        let mut reader = io::BufReader::with_capacity(3, &b"abcd\n"[..]);
        let mut buf = Vec::new();
        assert_eq!(
            read_line_capped(&mut reader, 4, &mut buf).unwrap(),
            Line::Complete
        );
        assert_eq!(buf, b"abcd");
    }
}
