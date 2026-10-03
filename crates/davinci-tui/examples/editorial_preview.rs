//! Offline contact sheet of actual Ratatui buffers, using existing UI fixtures.
//! Run: cargo run -p davinci-tui --example editorial_preview --offline > preview.html
use davinci_tui::davinci::{
    app, fixtures,
    model::{Model, Overlay, Screen},
    theme::{ColorDepth, Theme},
};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Paragraph, Widget},
};

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn css(color: Color) -> String {
    match color {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => "inherit".into(),
    }
}

fn main() {
    println!("<!doctype html><html lang=en><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><title>DaVinci terminal — rendered UI review</title><style>body{{background:#181818;color:#e6e6e6;margin:28px;font:15px system-ui,sans-serif}}h1{{font-size:26px;font-weight:600}}h2{{font-size:16px;margin-top:36px}}pre{{font:14px/1.45 ui-monospace,Consolas,monospace;overflow:auto;padding:18px;border:1px solid #555;border-radius:8px}}p{{color:#aaa;max-width:850px;line-height:1.6}}a{{color:#b1b9f9}}nav{{max-width:1100px;line-height:2}}</style><h1>DaVinci · terminal UI review</h1><p>Actual Ratatui frame output from deterministic offline fixtures. The tasks, paths and provider/account facts shown are sample data, not live jobs. No provider calls. This gallery does not certify a 1:1 visual match or native Windows Terminal behavior.</p>");
    let name = std::env::args().nth(1).unwrap_or_else(|| "dark".into());
    let cases = [
        ("1a", "Welcome", 120, 40, false),
        ("1b", "Conversation, tools and tasks", 120, 40, false),
        ("3a", "Model and effort selector", 120, 40, false),
        ("3b", "Settings", 120, 40, false),
        (
            "5a",
            "Optional graph and conversation input",
            120,
            40,
            false,
        ),
        (
            "blueprint",
            "Graph dependencies and inspector",
            160,
            50,
            false,
        ),
        ("1c", "Plan", 100, 30, false),
        ("1d", "Command palette", 100, 30, false),
        ("1e", "Code and changes split", 160, 40, false),
        ("1f", "Session overlay", 100, 30, false),
        ("1f-cogitator", "Quick model picker", 100, 30, false),
        ("1g", "Compact conversation", 80, 24, false),
        ("2a", "Code graph", 120, 40, false),
        ("2b", "Memory recall", 100, 30, false),
        ("2c", "Context budget", 100, 30, false),
        ("3c", "Effort levels", 100, 30, false),
        ("3d", "Provider authentication", 100, 30, false),
        ("3e", "Keyboard shortcuts", 120, 40, false),
        ("4a", "Session history", 120, 40, false),
        ("4b", "Session branches", 100, 30, false),
        ("4c", "Compaction", 100, 30, false),
        ("4d", "Export", 100, 30, false),
        ("5b", "Vector memory", 100, 30, false),
        ("5c", "Output governor", 100, 30, false),
        ("5d", "Security report", 100, 30, false),
        ("6a", "Project trust", 100, 30, false),
        ("6b", "Extensions", 100, 30, false),
        ("6c", "Interrupted-run recovery", 100, 30, false),
        ("6d", "Review changes", 120, 40, false),
        ("mcp", "MCP connections", 100, 30, false),
        ("permissions", "Permission policy", 120, 40, false),
        ("workflows", "Saved workflows", 100, 30, false),
        ("tasks", "Task board", 100, 30, false),
        ("agents", "Agent activity", 100, 30, false),
        ("context", "Context inspector", 100, 30, false),
        ("plugin", "Plugins, skills and MCP servers", 100, 30, false),
        ("plugin", "Plugins — narrow", 40, 20, false),
        ("secret", "Masked credential dialog", 100, 30, false),
        ("ask", "Question dialog", 100, 30, false),
        ("3b", "Settings — narrow", 40, 12, false),
        ("5a", "Graph — narrow", 40, 12, false),
        ("1a", "Monochrome", 80, 24, true),
    ];
    print!("<nav>");
    for (index, (_, title, ..)) in cases.iter().enumerate() {
        print!("<a href=#screen-{index}>{title}</a> · ");
    }
    println!("</nav>");
    for (index, (id, title, width, height, no_color)) in cases.into_iter().enumerate() {
        let theme = Theme::da_vinci(ColorDepth::TrueColor, no_color).with_name(&name);
        let mut model = Model::new(theme, width, height, false);
        fixtures::dress_screen(&mut model, id);
        if id == "secret" {
            model.screen = Screen::Agent;
            model.overlay = Some(Overlay::SecretInput);
            model.secret_input = Some(Default::default());
        } else if id == "ask" {
            model.screen = Screen::Agent;
            model.ask.title = "Continue this operation?".into();
            model.ask.note = "Fixture question. No operation will run.".into();
            model.ask.items = vec![
                davinci_tui::davinci::model::PickerItem::new("Continue", "Proceed once"),
                davinci_tui::davinci::model::PickerItem::new("Cancel", "Keep the current state"),
            ];
            model.overlay = Some(Overlay::Ask);
        }
        let screen = match id {
            "mcp" => Some(Screen::Mcp),
            "permissions" => Some(Screen::Permissions),
            "workflows" => Some(Screen::Workflows),
            "tasks" => Some(Screen::TaskBoard),
            "agents" => Some(Screen::Agents),
            "context" => Some(Screen::ContextInspector),
            _ => None,
        };
        if let Some(screen) = screen {
            model.screen = screen;
            model.overlay = None;
            model.transcript.clear();
        }
        model.width = width;
        model.height = height;
        let area = Rect::new(0, 0, width, height);
        let mut buffer = Buffer::empty(area);
        Paragraph::new(app::compose(&model, height))
            .style(Style::default().fg(theme.text).bg(theme.background))
            .render(area, &mut buffer);
        println!(
            "<h2 id=screen-{index}>{title} · {width}×{height}</h2><pre data-case=\"{id}\" style=\"background:{}\">",
            css(theme.background)
        );
        for y in 0..height {
            let mut x = 0;
            while x < width {
                let cell = &buffer[(x, y)];
                let style = cell.style();
                let mut text = String::new();
                while x < width && buffer[(x, y)].style() == style {
                    text.push_str(buffer[(x, y)].symbol());
                    x = x.saturating_add(
                        unicode_width::UnicodeWidthStr::width(buffer[(x, y)].symbol()).max(1)
                            as u16,
                    );
                }
                print!(
                    "<span style=\"color:{};background:{};font-weight:{}\">{}</span>",
                    css(cell.fg),
                    css(cell.bg),
                    if cell.modifier.contains(Modifier::BOLD) {
                        "bold"
                    } else {
                        "normal"
                    },
                    escape(&text)
                );
            }
            println!();
        }
        println!("</pre>");
    }
}
