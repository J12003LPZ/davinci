pub trait Component {
    fn render(&self, width: usize) -> Vec<String>;
}

pub struct Text {
    pub content: String,
}

impl Text {
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
        }
    }
}

impl Component for Text {
    fn render(&self, _width: usize) -> Vec<String> {
        self.content.lines().map(|s| s.to_string()).collect()
    }
}

pub struct Container {
    pub children: Vec<Box<dyn Component>>,
}

impl Default for Container {
    fn default() -> Self {
        Self::new()
    }
}

impl Container {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
        }
    }

    pub fn add(&mut self, child: Box<dyn Component>) {
        self.children.push(child);
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
}
