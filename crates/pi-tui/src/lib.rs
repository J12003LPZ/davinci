use anyhow::Result;
use pi_core::Message;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

pub struct TuiState {
    pub session_id: String,
    pub messages: Vec<Message>,
    pub input_buffer: String,
    pub is_streaming: bool,
}

impl TuiState {
    pub fn new(session_id: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            messages: Vec::new(),
            input_buffer: String::new(),
            is_streaming: false,
        }
    }

    pub fn draw(&self, frame: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(5),
                Constraint::Length(3),
            ])
            .split(frame.area());

        // Header
        let header = Paragraph::new(format!("Pi Agent - Session: {}", self.session_id))
            .style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .block(Block::default().borders(Borders::ALL).title("Header"));
        frame.render_widget(header, chunks[0]);

        // Message List
        let items: Vec<ListItem> = self
            .messages
            .iter()
            .map(|m| {
                let role_label = match m.role {
                    pi_core::Role::User => "[User]",
                    pi_core::Role::Assistant => "[Pi]",
                    pi_core::Role::System => "[System]",
                    pi_core::Role::Tool => "[Tool]",
                };
                let content = format!("{} {}", role_label, m.content);
                ListItem::new(content)
            })
            .collect();

        let message_list =
            List::new(items).block(Block::default().borders(Borders::ALL).title("Conversation"));
        frame.render_widget(message_list, chunks[1]);

        // Input
        let input = Paragraph::new(self.input_buffer.as_str())
            .style(Style::default().fg(Color::Yellow))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Input (Press Enter to Send)"),
            );
        frame.render_widget(input, chunks[2]);
    }
}
