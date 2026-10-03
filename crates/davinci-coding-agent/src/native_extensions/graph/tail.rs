//! Fixed-size diagnostic tail buffers.
use std::collections::VecDeque;

pub struct TailBuffer {
    cap: usize,
    buf: VecDeque<u8>,
}

impl TailBuffer {
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            buf: VecDeque::with_capacity(cap.min(4096)),
        }
    }

    pub fn push(&mut self, bytes: &[u8]) {
        if self.cap == 0 {
            return;
        }
        if bytes.len() >= self.cap {
            self.buf.clear();
            self.buf
                .extend(bytes[bytes.len() - self.cap..].iter().copied());
            return;
        }
        while self.buf.len() + bytes.len() > self.cap {
            self.buf.pop_front();
        }
        self.buf.extend(bytes.iter().copied());
    }

    pub fn text(&self) -> String {
        let (left, right) = self.buf.as_slices();
        let mut bytes = Vec::with_capacity(self.buf.len());
        bytes.extend_from_slice(left);
        bytes.extend_from_slice(right);
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_buffer_keeps_only_the_end() {
        let mut tail = TailBuffer::new(8);
        tail.push(b"0123456789");
        tail.push(b"ab");
        assert_eq!(tail.text(), "456789ab");
    }
}
