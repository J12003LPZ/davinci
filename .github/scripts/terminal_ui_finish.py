"""Follow-up to the recorded first CI run: fix real window/contrast defects,
then update only the presentation assertions intentionally superseded by the
approved redesign. No behavioral or safety tests are removed or disabled.
"""
from pathlib import Path

root = Path.cwd()
T = 'crates/davinci-tui/src/davinci/'

def edit(path, callback):
    p=root/path
    old=p.read_text()
    new=callback(old)
    if new == old: raise RuntimeError(f'No change: {path}')
    p.write_text(new)

def rep(s,a,b,n=1):
    if s.count(a)!=n: raise RuntimeError(f'Unexpected preimage: {a[:90]!r}')
    return s.replace(a,b)

def fn(s,signature,body):
    start=s.index(signature); at=s.index('{',start); depth=1; i=at+1
    while depth:
        depth+=(s[i]=='{')-(s[i]=='}'); i+=1
    return s[:start]+body+s[i:]

def test_text(s,replacements):
    at=s.index('#[cfg(test)]'); before=s[:at]; after=s[at:]
    for a,b in replacements.items(): after=after.replace(a,b)
    return before+after

# Recorded failure: Indexed(61) on Indexed(254) is 4.41. Do not lower 4.5.
edit(T+'theme.rs',lambda s:rep(s,'    primary: Color::Indexed(61),','    primary: Color::Indexed(60),'))

def config(s):
    s=rep(s,'''    rows = ui::window_section(rows, room, PINNED_DETAIL_ROWS, Some(anchor), &model.theme);''','''    let pinned = PINNED_DETAIL_ROWS.min(rows.len()).min(room.saturating_sub(1));
    let mut visible = rows.drain(..pinned).collect::<Vec<_>>();
    visible.extend(ui::window(rows, room.saturating_sub(pinned), anchor.saturating_sub(pinned), &model.theme));
    rows = visible;''')
    s=s.replace('  Type to filter · ↑↓ move · Enter change · Esc close','  Type to filter · ↑↓ move · enter change · esc close')
    return test_text(s,{'SETTING DETAILS':'Search settings…'})
edit(T+'views/settings.rs',config)

for path,mapping in {
    'views/ask.rs':{'PROJECT TRUST':'Project trust'},
    'views/instrumenta.rs':{'COMMANDS':'Commands'},
    'views/memoria.rs':{'RESUME SESSION':'Resume session'},
    'views/cogitator.rs':{'SELECT A MODEL':'Select a model','SELECT MODEL':'Select model'},
}.items():
    edit(T+path,lambda s,mapping=mapping:test_text(s,mapping))

def chrome(s):
    s=rep(s,'vec![paper_label("davinci", th, true)]','vec![paper_label("DaVinci", th, true)]')
    s=test_text(s,{'REVIEW CHANGES':'Review changes','SELECT A MODEL':'Select a model','DAVINCI':'DaVinci','"━╸┄╺".contains(ch)':"ch == '─'",'row.contains("26 ·")':'row.contains("26 commands")','            "LATEST",\n':''})
    return s
edit(T+'views/chrome.rs',chrome)

def app(s):
    s=test_text(s,{'SELECT A MODEL':'Select a model','SELECT MODEL':'Select model','SETTING DETAILS':'Search settings…','DAVINCI':'DaVinci','"━╸┄╺".contains(ch)':"ch == '─'"})
    s=fn(s,'fn a_sheet_starts_under_the_header_and_ends_with_its_hint_row()', '''fn a_sheet_starts_under_the_header_and_ends_with_its_hint_row() {
        let mut m = model(100, 44);
        crate::davinci::fixtures::dress_screen(&mut m, "3b");
        m.transcript = vec![Entry::user("keep this conversation visible")];
        let rows = compose(&m, 44);
        assert_eq!(rows.len(), 44);
        let title = rows.iter().position(|row| row_text(row).trim() == "Settings").unwrap();
        let context = rows.iter().position(|row| row_text(row).contains("keep this conversation visible")).unwrap();
        assert!(context < title);
        let hint = rows.iter().rev().nth(1).map(row_text).unwrap();
        assert!(hint.trim_end().ends_with("esc close"), "{hint}");
    }''')
    # Full-width headers are for sheets, not the new content-sized config picker.
    start=s.index('fn sheet_header_and_status_bar_fill_one_row_each_at_every_width()'); end=s.index('\n    #[test]',start)
    s=s[:start]+s[start:end].replace('Screen::Settings','Screen::Thinking')+s[end:]
    # Explicit usage shortcut moved off the shared editor's Ctrl+U.
    start=s.index('fn every_instrument_has_a_key_and_esc_closes_it()'); end=s.index('\n    #[test]',start)
    part=s[start:end].replace('handle_key(&mut m, ctrl(ch))',"handle_key(&mut m, KeyEvent::new(KeyCode::Char(ch), if ch == 'u' { KeyModifiers::CONTROL | KeyModifiers::ALT } else { KeyModifiers::CONTROL }))")
    return s[:start]+part+s[end:]
edit(T+'app.rs',app)

edit('crates/davinci-tui/src/themes.rs',lambda s:test_text(s,{
    '38;2;243;217;13':'38;2;177;185;249',
    '38;2;216;166;135':'38;2;217;155;130',
    '38;2;197;149;116':'38;2;170;170;170',
    '48;2;94;28;22':'48;2;43;43;43',
    '48;2;243;217;13':'48;2;177;185;249',
}))

# The former hard-coded LATEST badge is intentionally removed. Focus is now
# foreground emphasis, not a full-row colored background. Keep current vs
# focused identity assertions, and add an explicit no-badge assertion.
def catalog_test(s):
    s=test_text(s,{'            "LATEST",\n':'','s.style.bg == Some(m.theme.model_picker_colors().1)':'s.style.fg == Some(m.theme.model_picker_colors().0)'})
    return rep(s,'        let drawn = text(&screen(&m, 24));','        let drawn = text(&screen(&m, 24));\n        assert!(!drawn.contains("LATEST"));')
edit(T+'views/cogitator.rs',catalog_test)
print('Corrected config windowing, palette contrast, and old presentation assertions.')
