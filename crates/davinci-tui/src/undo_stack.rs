//! Generic undo stack matching TS `undo-stack.ts`.
//!
//! One deliberate divergence: the TS stack grows without limit, and every
//! push clones the paste store. Here the oldest snapshots fall off past
//! [`UNDO_CAP`] — nobody undoes two hundred steps, and an unbounded stack of
//! buffer clones is memory held for the life of the session.

/// How many snapshots are kept. Beyond this the oldest is dropped.
pub const UNDO_CAP: usize = 200;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UndoStack<S> {
    stack: Vec<S>,
}

impl<S> UndoStack<S> {
    pub fn new() -> Self {
        Self { stack: Vec::new() }
    }

    pub fn push(&mut self, state: S) {
        if self.stack.len() >= UNDO_CAP {
            self.stack.remove(0);
        }
        self.stack.push(state);
    }

    pub fn pop(&mut self) -> Option<S> {
        self.stack.pop()
    }

    pub fn clear(&mut self) {
        self.stack.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_pop_clear_and_len_match_ts() {
        let mut stack = UndoStack::new();
        stack.push("a");
        stack.push("b");
        assert_eq!(stack.pop(), Some("b"));
        stack.clear();
        assert_eq!(stack.pop(), None);
    }

    #[test]
    fn the_stack_is_bounded_and_drops_its_oldest_snapshot() {
        let mut stack = UndoStack::new();
        for n in 0..(UNDO_CAP + 10) {
            stack.push(n);
        }
        let mut drained = Vec::new();
        while let Some(state) = stack.pop() {
            drained.push(state);
        }
        assert_eq!(drained.len(), UNDO_CAP);
        assert_eq!(drained.first(), Some(&(UNDO_CAP + 9)));
        assert_eq!(drained.last(), Some(&10), "the oldest ten fell off");
    }
}
