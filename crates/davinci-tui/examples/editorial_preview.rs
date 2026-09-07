//! Offline contact sheet of actual Ratatui buffers, using existing UI fixtures.
//! Run: cargo run -p davinci-tui --example editorial_preview --offline > preview.html
use davinci_tui::davinci::{
    app, fixtures,
    model::Model,
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
    println!("<!doctype html><meta charset=utf-8><title>Davinci — editorial UI</title><style>body{{background:#1d1516;color:#d8a687;margin:32px;font:15px monospace}}h1{{font-size:26px}}h2{{font-size:15px;margin-top:32px}}pre{{font:14px/1.4 Consolas,monospace;overflow:auto;padding:16px;border:1px solid #9c6c4f}}p{{color:#b88564}}</style><h1>DAVINCI / EDITORIAL PRINT</h1><p>Offline fixture data · actual Ratatui cell colors and layout · no provider calls</p>");
    let name = std::env::args().nth(1).unwrap_or_else(|| "dark".into());
    for (id, title, width, height, no_color) in [
        ("1a", "01 / Welcome", 100, 25, false),
        ("1b", "02 / Conversation", 100, 32, false),
        ("3a", "03 / Model catalog", 100, 28, false),
        ("3b", "04 / Settings — narrow", 60, 24, false),
        ("5a", "05 / Graph run", 100, 30, false),
        ("6d", "06 / Review changes", 100, 30, false),
        ("1d", "07 / Command palette", 80, 24, false),
        ("1a", "08 / Monochrome", 60, 24, true),
    ] {
        let theme = Theme::da_vinci(ColorDepth::TrueColor, no_color).with_name(&name);
        let mut model = Model::new(theme, width, height, false);
        fixtures::dress_screen(&mut model, id);
        let area = Rect::new(0, 0, width, height);
        let mut buffer = Buffer::empty(area);
        Paragraph::new(app::compose(&model, height))
            .style(Style::default().fg(theme.text).bg(theme.background))
            .render(area, &mut buffer);
        println!(
            "<h2>{title} · {width}×{height}</h2><pre style=\"background:{}\">",
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
                    x += 1;
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
