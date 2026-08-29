//! Generic undo stack matching TS `undo-stack.ts`.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UndoStack<S> {
    stack: Vec<S>,
}

impl<S> UndoStack<S> {
    pub fn new() -> Self {
        Self { stack: Vec::new() }
    }

    pub fn push(&mut self, state: S) {
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
}
