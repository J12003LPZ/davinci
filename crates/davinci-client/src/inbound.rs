//! Platform-neutral pieces of the synchronous inbound pump used by the Unix
//! transport: which frame the bytes seen so far belong to, and when a pump
//! may stop waiting. Kept out of `unix.rs` so the timing rules run on every
//! platform's test suite.

use std::time::{Duration, Instant};

const HEADER_LENGTH: usize = 4;

/// Tracks the 4-byte big-endian length-prefixed framing of an inbound byte
/// stream, only far enough to tell whether a frame is currently split.
#[derive(Debug, Default, Clone)]
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) struct FrameWatch {
    header: [u8; HEADER_LENGTH],
    header_len: usize,
    payload_left: u64,
}

#[cfg_attr(not(unix), allow(dead_code))]
impl FrameWatch {
    pub(crate) fn observe(&mut self, mut chunk: &[u8]) {
        while !chunk.is_empty() {
            if self.payload_left > 0 {
                let take = chunk
                    .len()
                    .min(usize::try_from(self.payload_left).unwrap_or(usize::MAX));
                self.payload_left -= take as u64;
                chunk = &chunk[take..];
                continue;
            }
            let need = HEADER_LENGTH - self.header_len;
            let take = chunk.len().min(need);
            self.header[self.header_len..self.header_len + take].copy_from_slice(&chunk[..take]);
            self.header_len += take;
            chunk = &chunk[take..];
            if self.header_len == HEADER_LENGTH {
                self.payload_left = u64::from(u32::from_be_bytes(self.header));
                self.header_len = 0;
            }
        }
    }

    /// Some bytes of a frame have arrived and the rest have not.
    pub(crate) fn mid_frame(&self) -> bool {
        self.header_len > 0 || self.payload_left > 0
    }
}

/// How long a pump waits once data has started to arrive.
pub(crate) const QUIET_WINDOW: Duration = Duration::from_millis(20);

/// Whether the pump may return.
///
/// Between frames, a quiet window after the last data means the peer has
/// said what it had to say, and `timeout` bounds the whole wait. A frame that
/// is split is a valid message still in flight, however slowly its chunks
/// are delivered: the quiet window does not apply, and only a stall of
/// `timeout` since the last chunk gives up.
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) fn pump_finished(
    now: Instant,
    started: Instant,
    last_data: Instant,
    got_any: bool,
    mid_frame: bool,
    timeout: Duration,
) -> bool {
    if mid_frame {
        return now.saturating_duration_since(last_data) > timeout;
    }
    if now.saturating_duration_since(started) > timeout {
        return true;
    }
    got_any && now.saturating_duration_since(last_data) > QUIET_WINDOW
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(payload: &[u8]) -> Vec<u8> {
        let mut out = (payload.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn a_split_frame_is_mid_frame_until_its_last_byte() {
        let bytes = frame(b"hello world");
        let mut watch = FrameWatch::default();
        assert!(!watch.mid_frame());
        for (index, byte) in bytes.iter().enumerate() {
            watch.observe(std::slice::from_ref(byte));
            assert_eq!(watch.mid_frame(), index + 1 < bytes.len(), "byte {index}");
        }
    }

    #[test]
    fn chunk_boundaries_do_not_change_the_answer() {
        let mut stream = frame(b"one");
        stream.extend(frame(b""));
        stream.extend(frame(&[7u8; 300]));
        // Offsets where one frame ends and the next begins: "one" takes 7
        // bytes, the empty frame 4 more.
        let boundaries = [7, 11, stream.len()];
        for split in 1..stream.len() {
            let mut watch = FrameWatch::default();
            watch.observe(&stream[..split]);
            assert_eq!(
                watch.mid_frame(),
                !boundaries.contains(&split),
                "split {split}"
            );
            watch.observe(&stream[split..]);
            assert!(!watch.mid_frame(), "split {split}");
        }
    }

    #[test]
    fn several_frames_in_one_chunk_end_between_frames() {
        let mut stream = frame(b"a");
        stream.extend(frame(b"bc"));
        let mut watch = FrameWatch::default();
        watch.observe(&stream);
        assert!(!watch.mid_frame());
        watch.observe(&frame(b"xyz")[..5]);
        assert!(watch.mid_frame());
    }

    #[test]
    fn a_partial_header_counts_as_mid_frame() {
        let mut watch = FrameWatch::default();
        watch.observe(&[0, 0]);
        assert!(watch.mid_frame());
        watch.observe(&[0, 0]);
        assert!(!watch.mid_frame(), "a zero-length frame is complete");
    }

    fn at(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    /// WOR-47: gaps well past the quiet window inside one frame must not end
    /// the pump.
    #[test]
    fn a_slow_partial_frame_outlives_the_quiet_window() {
        let base = Instant::now();
        let timeout = Duration::from_secs(2);
        // Data arrived at t=0 and the frame is still split at t=500.
        assert!(!pump_finished(
            at(base, 500),
            base,
            base,
            true,
            true,
            timeout
        ));
        assert!(!pump_finished(
            at(base, 1999),
            base,
            base,
            true,
            true,
            timeout
        ));
        // Even past the overall timeout, as long as chunks keep arriving.
        assert!(!pump_finished(
            at(base, 5000),
            base,
            at(base, 4900),
            true,
            true,
            timeout
        ));
        // A real stall gives up.
        assert!(pump_finished(
            at(base, 4001),
            base,
            base,
            true,
            true,
            timeout
        ));
    }

    #[test]
    fn between_frames_a_quiet_window_or_the_timeout_ends_the_pump() {
        let base = Instant::now();
        let timeout = Duration::from_secs(2);
        assert!(!pump_finished(
            at(base, 10),
            base,
            base,
            true,
            false,
            timeout
        ));
        assert!(pump_finished(
            at(base, 21),
            base,
            base,
            true,
            false,
            timeout
        ));
        // No data yet: only the timeout ends it.
        assert!(!pump_finished(
            at(base, 500),
            base,
            base,
            false,
            false,
            timeout
        ));
        assert!(pump_finished(
            at(base, 2001),
            base,
            base,
            false,
            false,
            timeout
        ));
    }
}
