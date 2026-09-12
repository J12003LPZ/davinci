use std::sync::atomic::{AtomicU64, Ordering};

pub fn next(sequence: &AtomicU64) -> u64 {
    sequence.load(Ordering::Relaxed) + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consecutive_allocations_are_unique() {
        let sequence = AtomicU64::new(0);
        assert_eq!(next(&sequence), 1);
        assert_eq!(next(&sequence), 2);
    }
}
