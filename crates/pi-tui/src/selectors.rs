use crate::component::Component;
use crate::fuzzy::fuzzy_filter;

#[derive(Debug, Clone)]
pub struct SelectList {
    pub items: Vec<String>,
    pub selected: usize,
    pub query: String,
}

impl SelectList {
    pub fn new(items: Vec<String>) -> Self {
        Self {
            items,
            selected: 0,
            query: String::new(),
        }
    }

    pub fn filtered(&self) -> Vec<String> {
        fuzzy_filter(&self.query, &self.items)
            .into_iter()
            .map(|m| m.item)
            .collect()
    }
}

impl Component for SelectList {
    fn render(&self, width: usize) -> Vec<String> {
        self.filtered()
            .into_iter()
            .enumerate()
            .map(|(i, item)| {
                let prefix = if i == self.selected { "> " } else { "  " };
                let mut line = format!("{prefix}{item}");
                if line.len() > width {
                    line.truncate(width);
                }
                line
            })
            .collect()
    }
}
