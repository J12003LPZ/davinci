//! Container matching `vendor/pi/packages/tui/src/tui.ts`.

use std::cell::RefCell;
use std::rc::Rc;

use crate::render::{Component, RenderedLines};

pub struct Container {
    pub children: Vec<Box<dyn Component>>,
}

impl Container {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
        }
    }

    pub fn add_child(&mut self, component: Box<dyn Component>) {
        self.children.push(component);
    }

    pub fn remove_child_at(&mut self, index: usize) {
        if index < self.children.len() {
            self.children.remove(index);
        }
    }

    pub fn clear(&mut self) {
        self.children.clear();
    }
}

impl Default for Container {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared children used as the alt-screen implicit document.
pub struct SharedContainer {
    pub inner: Rc<RefCell<Container>>,
}

impl SharedContainer {
    pub fn new(inner: Rc<RefCell<Container>>) -> Self {
        Self { inner }
    }
}

impl Component for SharedContainer {
    fn render(&self, width: usize) -> Vec<String> {
        self.inner.borrow().render(width)
    }

    fn rendered_lines(&self, width: usize) -> RenderedLines {
        self.inner.borrow().rendered_lines(width)
    }

    fn handle_input(&mut self, data: &str) {
        self.inner.borrow_mut().handle_input(data);
    }

    fn invalidate(&mut self) {
        self.inner.borrow_mut().invalidate();
    }

    fn wants_key_release(&self) -> bool {
        self.inner.borrow().wants_key_release()
    }
}

/// Cloneable component handle matching TS remount of the same child object.
pub struct SharedComponent {
    inner: Rc<RefCell<dyn Component>>,
}

impl SharedComponent {
    pub fn new(component: impl Component + 'static) -> Self {
        Self {
            inner: Rc::new(RefCell::new(component)),
        }
    }

    pub fn inner(&self) -> Rc<RefCell<dyn Component>> {
        self.inner.clone()
    }

    pub fn ptr_eq(left: &Self, right: &Self) -> bool {
        Rc::ptr_eq(&left.inner, &right.inner)
    }
}

impl Clone for SharedComponent {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl Component for SharedComponent {
    fn render(&self, width: usize) -> Vec<String> {
        self.inner.borrow().render(width)
    }

    fn rendered_lines(&self, width: usize) -> RenderedLines {
        self.inner.borrow().rendered_lines(width)
    }

    fn handle_input(&mut self, data: &str) {
        self.inner.borrow_mut().handle_input(data);
    }

    fn invalidate(&mut self) {
        self.inner.borrow_mut().invalidate();
    }

    fn wants_key_release(&self) -> bool {
        self.inner.borrow().wants_key_release()
    }
}

impl Component for Container {
    fn render(&self, width: usize) -> Vec<String> {
        let mut lines = Vec::new();
        for child in &self.children {
            lines.extend(child.render(width));
        }
        lines
    }

    fn handle_input(&mut self, data: &str) {
        if let Some(child) = self.children.last_mut() {
            child.handle_input(data);
        }
    }

    fn invalidate(&mut self) {
        for child in &mut self.children {
            child.invalidate();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spacer::Spacer;
    use crate::tui_text::TuiText;

    #[test]
    fn container_stacks_child_lines() {
        let mut root = Container::new();
        root.add_child(Box::new(TuiText::new("one", 0, 0)));
        root.add_child(Box::new(Spacer::new(1)));
        root.add_child(Box::new(TuiText::new("two", 0, 0)));
        let lines: Vec<String> = root
            .render(8)
            .into_iter()
            .map(|line| line.trim_end().to_string())
            .collect();
        assert_eq!(lines, vec!["one", "", "two"]);
    }
}
