use crate::render::{visible_width, Component};

/// Box component matching `vendor/pi/packages/tui/src/components/box.ts`.
pub struct TuiBox {
    pub padding_x: usize,
    pub padding_y: usize,
    children: Vec<std::boxed::Box<dyn Component>>,
}

impl TuiBox {
    pub fn new(padding_x: usize, padding_y: usize) -> Self {
        Self {
            padding_x,
            padding_y,
            children: Vec::new(),
        }
    }

    pub fn add_child(&mut self, child: std::boxed::Box<dyn Component>) {
        self.children.push(child);
    }
}

impl Component for TuiBox {
    fn render(&self, width: usize) -> Vec<String> {
        let inner_width = width.saturating_sub(self.padding_x.saturating_mul(2));
        let pad = " ".repeat(self.padding_x);
        let mut lines: Vec<String> = (0..self.padding_y).map(|_| " ".repeat(width)).collect();
        for child in &self.children {
            for line in child.render(inner_width) {
                let mut out = format!("{pad}{line}");
                while visible_width(&out) < width {
                    out.push(' ');
                }
                lines.push(out);
            }
        }
        for _ in 0..self.padding_y {
            lines.push(" ".repeat(width));
        }
        lines
    }

    fn invalidate(&mut self) {
        for child in &mut self.children {
            child.invalidate();
        }
    }
}
