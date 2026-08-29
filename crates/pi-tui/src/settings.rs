use crate::fuzzy::fuzzy_filter;
use crate::render::Component;

#[derive(Debug, Clone)]
pub struct SettingItem {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub current_value: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SettingsList {
    pub items: Vec<SettingItem>,
    pub selected: usize,
    pub query: String,
    pub max_visible: usize,
}

impl SettingsList {
    pub fn new(items: Vec<SettingItem>, max_visible: usize) -> Self {
        Self {
            items,
            selected: 0,
            query: String::new(),
            max_visible,
        }
    }

    pub fn cycle(&mut self) {
        if let Some(item) = self.filtered().get(self.selected).cloned() {
            if let Some(item) = self.items.iter_mut().find(|i| i.id == item.id) {
                if item.values.is_empty() {
                    return;
                }
                let current = item
                    .values
                    .iter()
                    .position(|v| v == &item.current_value)
                    .unwrap_or(0);
                item.current_value = item.values[(current + 1) % item.values.len()].clone();
            }
        }
    }

    fn filtered(&self) -> Vec<SettingItem> {
        if self.query.is_empty() {
            return self.items.clone();
        }
        let labels: Vec<String> = self.items.iter().map(|i| i.label.clone()).collect();
        let kept = fuzzy_filter(&self.query, &labels);
        self.items
            .iter()
            .filter(|item| kept.contains(&item.label))
            .cloned()
            .collect()
    }
}

impl Component for SettingsList {
    fn render(&self, width: usize) -> Vec<String> {
        let filtered = self.filtered();
        filtered
            .iter()
            .take(self.max_visible)
            .enumerate()
            .map(|(index, item)| {
                let prefix = if index == self.selected { "> " } else { "  " };
                let line = format!("{prefix}{}  {}", item.label, item.current_value);
                if line.len() > width {
                    line.chars().take(width).collect()
                } else {
                    line
                }
            })
            .collect()
    }

    fn handle_input(&mut self, data: &str) {
        if data == " " || data == "\n" {
            self.cycle();
        } else {
            self.query.push_str(data);
        }
    }

    fn invalidate(&mut self) {}
}
