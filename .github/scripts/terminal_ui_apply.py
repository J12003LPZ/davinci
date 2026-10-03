"""Apply the reviewed terminal-UI batch in the isolated branch CI checkout.

No network, credentials, or runtime settings are accessed. Exact preimage checks
fail rather than modifying a changed implementation. The workflow tests and
commits actual Rust source; this file is not a substitute for product changes.
"""
from pathlib import Path
import re

root = Path.cwd()
T = 'crates/davinci-tui/src/davinci/'

def edit(path, callback):
    p = root / path
    old = p.read_text()
    new = callback(old)
    if new == old:
        raise RuntimeError(f'No change: {path}')
    p.write_text(new)

def rep(s, old, new, n=1):
    if s.count(old) != n:
        raise RuntimeError(f'Unexpected preimage count for {old[:90]!r}: {s.count(old)}')
    return s.replace(old, new)

def fn(s, signature, body):
    start = s.index(signature)
    at = s.index('{', start)
    depth, i = 1, at + 1
    # These reviewed function bodies have balanced braces in strings/comments.
    while depth:
        depth += (s[i] == '{') - (s[i] == '}')
        i += 1
    return s[:start] + body + s[i:]

def theme(s):
    keys = ['background','surface','surface_alt','border','text','muted','primary','secondary','success','warning','error']
    ramps = {
        'TRUECOLOR': ['1F1F1F','2B2B2B','262626','767676','E6E6E6','AAAAAA','B1B9F9','D99B82','9BCC8B','E5C07B','F38B8B'],
        'LIGHT': ['FFFFFF','F5F5F5','EBEBEB','808080','202020','595959','4948A0','8A422D','28652E','745007','A82A35'],
        'TRUECOLOR_DIM': ['1F1F1F','262626','202020','444444','808080','707070','787CA1','856C61','6B8163','8A795B','987070'],
    }
    for name, values in ramps.items():
        new = f'const {name}: Ramp = Ramp {{\n' + ''.join(f'    {k}: rgb(0x{v}),\n' for k,v in zip(keys,values)) + '};'
        s, count = re.subn(r'const '+name+r': Ramp = Ramp \{.*?\n\};', lambda _:new, s, count=1, flags=re.S)
        assert count == 1
    for name, values in {
        'ANSI256': [234,235,235,243,254,248,147,180,150,180,210],
        'ANSI256_DIM': [234,235,234,238,244,242,103,137,65,137,138],
        'LIGHT_256': [231,255,254,244,234,240,61,94,22,58,124],
    }.items():
        new = f'const {name}: Ramp = Ramp {{\n' + ''.join(f'    {k}: Color::Indexed({v}),\n' for k,v in zip(keys,values)) + '};'
        s, count = re.subn(r'const '+name+r': Ramp = Ramp \{.*?\n\};', lambda _:new, s, count=1, flags=re.S)
        assert count == 1
    s = s.replace('Color::Rgb(234, 213, 185)', 'Color::Rgb(255, 255, 255)').replace('| Color::Indexed(223)', '| Color::Indexed(231)')
    s = fn(s, 'fn truecolor_tokens_match_the_spec_table()', '''fn truecolor_tokens_match_the_spec_table() {
        let theme = Theme::da_vinci(ColorDepth::TrueColor, false);
        assert_eq!(theme.background, rgb(0x1F1F1F));
        assert_eq!(theme.surface, rgb(0x2B2B2B));
        assert_eq!(theme.surface_alt, rgb(0x262626));
        assert_eq!(theme.border, rgb(0x767676));
        assert_eq!(theme.text, rgb(0xE6E6E6));
        assert_eq!(theme.muted, rgb(0xAAAAAA));
        assert_eq!(theme.primary, rgb(0xB1B9F9));
        assert_eq!(theme.secondary, rgb(0xD99B82));
        assert_eq!(theme.success, rgb(0x9BCC8B));
        assert_eq!(theme.warning, rgb(0xE5C07B));
        assert_eq!(theme.error, rgb(0xF38B8B));
    }''')
    s = s.replace('assert_eq!(theme.primary, Color::Indexed(220));', 'assert_eq!(theme.primary, Color::Indexed(147));').replace('assert_eq!(theme.border, Color::Indexed(137));', 'assert_eq!(theme.border, Color::Indexed(243));')
    for old,new in [('Color::Rgb(0x80, 0x60, 0x4D)','rgb(0x808080)'),('Color::Rgb(0x70, 0x50, 0x3E)','rgb(0x707070)'),('Color::Rgb(0x82, 0x75, 0x22)','rgb(0x787CA1)'),('Color::Rgb(0x4A, 0x30, 0x28)','rgb(0x444444)')]:
        s=s.replace(old,new)
    return s

edit(T+'theme.rs', theme)

def ui(s):
    s = fn(s, 'pub fn paper_label(', '''pub fn paper_label(text: &str, theme: &Theme, accent: bool) -> Span<'static> {
    Span::styled(text.to_string(), Style::default()
        .fg(if accent { theme.primary } else { theme.text })
        .add_modifier(Modifier::BOLD))
}''')
    s = fn(s, 'pub fn print_rule(', '''pub fn print_rule(width: u16, theme: &Theme) -> Line<'static> {
    Line::from(span("─".repeat(usize::from(width)), theme.border))
}''')
    s = fn(s, 'pub fn clip(', '''pub fn clip(text: &str, max: u16) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    let mut out = String::new();
    let mut used = 0usize;
    for grapheme in text.graphemes(true) {
        let cells = UnicodeWidthStr::width(grapheme);
        if used.saturating_add(cells) > usize::from(max) { break; }
        out.push_str(grapheme);
        used = used.saturating_add(cells);
    }
    out
}''')
    s = fn(s, 'pub fn run_width(', '''pub fn run_width(spans: &[Span<'_>]) -> u16 {
    spans.iter().fold(0usize, |used, item| used.saturating_add(
        UnicodeWidthStr::width(item.content.as_ref())
    )).min(usize::from(u16::MAX)) as u16
}''')
    s = fn(s, 'pub fn title(mut self,', '''pub fn title(mut self, title: Vec<Span<'static>>) -> Self {
        self.title = title.into_iter().map(|mut item| {
            item.style = item.style.add_modifier(Modifier::BOLD);
            item
        }).collect();
        self
    }''')
    s = s.replace('    paper: Color,\n    ink: Color,\n', '').replace('            paper: theme.text,\n            ink: theme.background,\n', '')
    s = fn(s, 'fn editorial_labels_use_opaque_ink_and_preserve_monochrome()', '''fn editorial_labels_use_opaque_ink_and_preserve_monochrome() {
        use super::*;
        use crate::davinci::theme::ColorDepth;
        for depth in [ColorDepth::TrueColor, ColorDepth::Ansi256, ColorDepth::Basic] {
            for no_color in [false, true] {
                let theme = Theme::da_vinci(depth, no_color);
                let label = paper_label("Review changes", &theme, true);
                assert_eq!(label.content, "Review changes");
                assert!(label.style.add_modifier.contains(Modifier::BOLD));
                assert_eq!(label.style.fg, Some(theme.primary));
                assert!(label.style.bg.is_none());
            }
        }
    }''')
    return rep(s, '    let available = width.saturating_sub(3);', '    let width = width.min(96);\n    let available = width.saturating_sub(3);')

edit(T+'ui.rs', ui)
edit('crates/davinci-tui/src/keybindings.rs', lambda s:rep(s, '("davinci.mensura.toggle", &["ctrl+u"])', '("davinci.mensura.toggle", &["ctrl+alt+u"])'))

def legacy(s):
    dark={'#5E1C16':'#2B2B2B','#9C6C4F':'#767676','#D8A687':'#E6E6E6','#C59574':'#AAAAAA','#F3D90D':'#B1B9F9','#E6A080':'#F38B8B'}
    at=s.index('impl Palette'); end=s.index('#[derive(Debug, Clone, Serialize',at)
    part=s[at:end]
    for a,b in dark.items(): part=part.replace(a,b)
    part=part.replace('secondary: "#E6E6E6"','secondary: "#D99B82"').replace('success: "#E6E6E6"','success: "#9BCC8B"').replace('warning: "#B1B9F9"','warning: "#E5C07B"')
    s=s[:at]+part+s[end:]
    at=s.index('pub fn builtin_themes'); end=s.index('name: "vox"',at)
    part=s[at:end]
    for a,b in {**dark,'#1D1516':'#1F1F1F','#EAD5B9':'#FFFFFF','#8D150F':'#4948A0','#E2BE9E':'#F5F5F5','#543829':'#595959','#182033':'#28652E'}.items(): part=part.replace(a,b)
    part=part.replace('foreground: "#1F1F1F"','foreground: "#202020"').replace('text: "#1F1F1F"','text: "#202020"').replace('secondary: "#28652E"','secondary: "#8A422D"').replace('warning: "#2B2B2B"','warning: "#745007"').replace('error: "#4948A0"','error: "#A82A35"')
    return s[:at]+part+s[end:]

edit('crates/davinci-tui/src/themes.rs', legacy)

def startup(s):
    s=s.replace('use ratatui::style::{Modifier, Stylize};\n','')
    s=fn(s,'pub fn banner(','''pub fn banner(model: &Model, info: &Startup) -> Vec<Line<'static>> {
    let th = &model.theme;
    let facts = [
        vec![span("✻  ", th.secondary), paper_label("DaVinci", th, false),
             span(format!(" v{}", env!("CARGO_PKG_VERSION")), th.muted)],
        vec![span("   ", th.muted), span(model.model_name.clone(), th.text),
             span(format!(" · {}", model.thinking_level), th.muted)],
        vec![span("   ", th.muted), span(info.cwd.clone(), th.muted)],
    ];
    facts.into_iter().map(|row| indent(1.min(model.width),
        truncate_run(row, model.width.saturating_sub(1)))).collect()
}''')
    s=fn(s,'pub fn lines(','''pub fn lines(model: &Model, info: &Startup) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let mut out = vec![blank()];
    out.extend(banner(model, info));
    out.push(blank());
    let mut rows = vec![Line::from(restored_row(th, info.restored))];
    for found in &info.found {
        rows.push(Line::from(span(found.clone(), th.muted)));
    }
    rows.push(Line::from(span("/help for commands · /model to change models", th.muted)));
    out.extend(rows.into_iter().map(|row| indent(1.min(width),
        truncate_run(row.spans, width.saturating_sub(1)))));
    out.push(blank());
    out
}''')
    s=fn(s,'pub fn height(','''pub fn height(model: &Model) -> usize {
    lines(model, &model.startup).len()
}''')
    s=s.replace('''        for command in [
            "/graph",
            "/governor-status",
            "/memory-status",
            "/model",
            "/resume",
            "/help",
        ]''','''        for command in ["/model", "/help"]''')
    return fn(s,'fn banner_uses_editorial_masthead()', '''fn banner_uses_editorial_masthead() {
        let m = model(100);
        let rows = banner(&m, &m.startup);
        let art = rows.iter().map(text).collect::<Vec<_>>().join("\\n");
        assert_eq!(rows.len(), 3);
        assert!(art.contains("DaVinci"));
        assert!(art.contains(&m.model_name));
        assert!(!art.contains("▓"));
        assert!(!art.contains("CODE / TOOLS / CONTEXT"));
    }''')

edit(T+'views/startup.rs', startup)

def chrome(s):
    old='''    Surface::new(model.width, th)
        .border(th.border)
        .title(vec![span("COMPLETIONS", th.border)])
        .right(vec![span(
            format!("{total} · ↑↓ move · tab take · esc close"),
            th.border,
        )])
        .rows(rows)
        .lines()'''
    new='''    let mut out = vec![crate::davinci::ui::blank()];
    out.extend(rows.into_iter().map(|row| Line::from(
        crate::davinci::ui::truncate_run(row, model.width))));
    out.push(Line::from(span(
        clip_ellipsis(&format!("   {total} commands · ↑↓ move · tab take · esc close"), model.width),
        th.muted,
    )));
    out'''
    s=rep(s,old,new)
    return rep(s,'''                            th.border,
                            tint,
                        ));''','''                            th.muted,
                            tint,
                        ));''')

edit(T+'views/chrome.rs', chrome)

def model(s):
    s=rep(s,'    pub catalog_index: usize,','    pub catalog_index: usize,\n    pub catalog_query: String,')
    s=rep(s,'    pub settings_index: usize,','    pub settings_index: usize,\n    pub settings_query: String,')
    s=rep(s,'            catalog_index: 0,','            catalog_index: 0,\n            catalog_query: String::new(),')
    return rep(s,'            settings_index: 0,','            settings_index: 0,\n            settings_query: String::new(),')

edit(T+'model.rs', model)
picker=root/(T+'views/picker.rs')
assert not picker.exists()
picker.write_text('''//! Shared search/navigation for native selectors. Source collections never move.
pub fn matches(query: &str, values: &[&str]) -> bool {
    let text = values.join(" ").to_lowercase();
    query.split_whitespace().all(|word| text.contains(&word.to_lowercase()))
}

pub fn step(indices: &[usize], selected: usize, delta: isize) -> usize {
    if indices.is_empty() { return selected; }
    let at = indices.iter().position(|index| *index == selected).unwrap_or(0);
    indices[crate::davinci::model::wrap_index(at, delta, indices.len())]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn search_handles_unicode_words_and_keeps_original_identity() {
        assert!(matches("MODEL b", &["Model", "provider-b"]));
        assert!(matches("界", &["Project 界"]));
        assert!(!matches("other", &["Model"]));
        assert_eq!(step(&[2, 7, 11], 7, 1), 11);
        assert_eq!(step(&[2, 7, 11], 2, -1), 11);
        assert_eq!(step(&[], 7, 1), 7);
    }
}
''')
edit(T+'views/mod.rs',lambda s:rep(s,'pub mod permissions;','pub mod permissions;\npub mod picker;'))

def settings(s):
    for old,new in [('GENERAL','General'),('DISPLAY','Display'),('AUTOCOMPLETE','Autocomplete'),('AGENT BEHAVIOR','Agent behavior'),('NETWORK','Network'),('ADVANCED','Advanced')]:
        s=s.replace('"'+old+'"','"'+new+'"')
    at=s.index('pub fn lines(')
    s=s[:at]+'''/// Visible source indices, including keys and descriptions in the search corpus.
pub fn visible_indices(model: &Model) -> Vec<usize> {
    model.settings_rows.iter().enumerate().filter_map(|(index, row)| {
        super::picker::matches(&model.settings_query, &[&row.label, &row.key, &row.description,
            group_label(group_rank(&row.key))]).then_some(index)
    }).collect()
}

/// Compact configuration panel. It occupies only its content, not the entire terminal.
pub fn screen(model: &Model, height: usize) -> Vec<Line<'static>> {
    let mut rows = lines(model);
    let anchor = model.section_offset.or_else(|| ui::focused_row(&rows)).unwrap_or(0);
    let room = height.saturating_sub(3);
    rows = ui::window_section(rows, room, PINNED_DETAIL_ROWS, Some(anchor), &model.theme);
    let mut out = vec![ui::indent(2.min(model.width), vec![ui::paper_label("Settings", &model.theme, true)])];
    out.extend(rows);
    out.push(Line::from(span(ui::clip_ellipsis("  Type to filter · ↑↓ move · Enter change · Esc close", model.width), model.theme.muted)));
    out.push(ui::blank());
    out.truncate(height);
    out
}

pub fn screen_height(model: &Model) -> usize {
    (lines(model).len() + 3).min((model.height as usize * 3 / 4).max(12))
}

'''+s[at:]
    s=rep(s,'''    let selected = model.settings_index % model.settings_rows.len();''','''    let indices = visible_indices(model);
    if indices.is_empty() {
        return vec![detail_line(model, format!("Search: {}", model.settings_query), true),
            detail_line(model, "No matching settings. Backspace to edit the search.", false)];
    }
    let selected = if indices.contains(&model.settings_index) { model.settings_index } else { indices[0] };''')
    s=s.replace('detail_line(model, "SETTING DETAILS", true),','detail_line(model, if model.settings_query.is_empty() { "Search settings…".into() } else { format!("Search: {}", model.settings_query) }, true),')
    return rep(s,'''    for (index, setting) in model.settings_rows.iter().enumerate() {
        let rank''','''    for index in indices {
        let setting = &model.settings_rows[index];
        let rank''')

edit(T+'views/settings.rs', settings)

def models(s):
    at=s.index('/// The catalog keeps source order')
    s=s[:at]+'''pub fn visible_indices(model: &Model) -> Vec<usize> {
    model.catalog.iter().enumerate().filter_map(|(index, row)| {
        super::picker::matches(&model.catalog_query, &[&row.name, &row.id, &row.provider])
            .then_some(index)
    }).collect()
}

'''+s[at:]
    s=rep(s,'    let selected = model.catalog_index % model.catalog.len();','''    let indices = visible_indices(model);
    if indices.is_empty() {
        return section_detail(model.width, th, "No matching models. Backspace to edit the search.");
    }
    let selected = if indices.contains(&model.catalog_index) { model.catalog_index } else { indices[0] };''')
    s=rep(s,'    for (index, entry) in model.catalog.iter().enumerate() {','    for index in indices {\n        let entry = &model.catalog[index];')
    s=s.replace('        section_offset: None,\n        section_notice: None,','        section_offset: None,\n        section_notice: None,\n        catalog_query: String::new(),')
    s=s.replace('"↑↓ model · ←→ reasoning · Enter saves"','"Type to filter · ↑↓ model · ←→ reasoning · Enter saves"')
    s=s.replace('''    content.push(ui::print_rule(width, th));''','''    content.push(Line::from(span(if model.catalog_query.is_empty() {
        "Search models…".to_string()
    } else { format!("Search: {}", model.catalog_query) }, th.muted)));''')
    s=s.replace('    let badge = entry.id == "gpt-6-astra" && width >= 65;','    let badge = false;')
    s=s.replace('    let (accent, background) = th.model_picker_colors();','    let (accent, _background) = th.model_picker_colors();')
    return s.replace('            item.style.bg = Some(background);','            item.style.fg = Some(accent);')

edit(T+'views/cogitator.rs', models)

def app(s):
    s=rep(s,'''    if model.screen == Screen::Models && model.overlay.is_none() {
        let picker = cogitator::screen(model, cogitator::screen_height(model).min(height));''','''    if matches!(model.screen, Screen::Models | Screen::Settings) && model.overlay.is_none() {
        let picker = if model.screen == Screen::Settings {
            settings::screen(model, settings::screen_height(model).min(height))
        } else {
            cogitator::screen(model, cogitator::screen_height(model).min(height))
        };''')
    pos=s.index('    let toggle_action = match model.screen')
    s=s[:pos]+'''    if handle_picker_search(model, &key) {
        return Flow::Continue;
    }

'''+s[pos:]
    pos=s.index('fn screen_move(')
    s=s[:pos]+'''/// Search input belongs to the selector; it must never overwrite the chat draft.
fn handle_picker_search(model: &mut Model, key: &KeyEvent) -> bool {
    if !matches!(model.screen, Screen::Models | Screen::Settings)
        || key.kind == KeyEventKind::Release
        || key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
        return false;
    }
    let query = if model.screen == Screen::Models { &mut model.catalog_query } else { &mut model.settings_query };
    match key.code {
        KeyCode::Char(ch) => query.push(ch),
        KeyCode::Backspace => {
            use unicode_segmentation::UnicodeSegmentation;
            let start = query.grapheme_indices(true).last().map(|(at, _)| at).unwrap_or(0);
            query.truncate(start);
        }
        _ => return false,
    }
    let (indices, selected) = if model.screen == Screen::Models {
        (cogitator::visible_indices(model), &mut model.catalog_index)
    } else { (settings::visible_indices(model), &mut model.settings_index) };
    if !indices.contains(selected) {
        if let Some(first) = indices.first() { *selected = *first; }
    }
    model.section_offset = None;
    true
}

'''+s[pos:]
    s=rep(s,'            model.catalog_index = wrap_index(model.catalog_index, delta, model.catalog.len());','            model.catalog_index = super::views::picker::step(&cogitator::visible_indices(model), model.catalog_index, delta);')
    s=rep(s,'''            model.settings_index =
                wrap_index(model.settings_index, delta, model.settings_rows.len());''','''            model.settings_index = super::views::picker::step(&settings::visible_indices(model), model.settings_index, delta);''')
    s=rep(s,'        Screen::Models => pick(model.catalog_index, model.catalog.len()).map(Choice::Catalog),','''        Screen::Models => cogitator::visible_indices(model).contains(&model.catalog_index)
            .then_some(Choice::Catalog(model.catalog_index)),''')
    s=rep(s,'            pick(model.settings_index, model.settings_rows.len()).map(Choice::Setting)','''            settings::visible_indices(model).contains(&model.settings_index)
                .then_some(Choice::Setting(model.settings_index))''')
    return s.replace('several of them (`ctrl+u` mensura, `ctrl+b` codex, `ctrl+d` quit)', 'some of them (explicit surface shortcuts and `ctrl+d` quit)')

edit(T+'app.rs', app)
print('Applied terminal shell, shared primitives, welcome, search and selector batch.')
