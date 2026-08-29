//! Composition root matching TS `createInteractiveTui`.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use pi_tui::{
    copy_text, open_browser, Component, MemoryTerminal, OverlayHandle, OverlayOptions,
    ProcessTerminal, ScrollFollow, ScrollView, ScrollViewOptions, StackBasis, StackEntryOptions,
    TerminalIo, Theme, TuiAltScreen, TuiAltScreenOptions, TuiMainScreen, TuiMainScreenRenderState,
    TuiMode, TuiRuntimeMode, TuiStopOptions, VStack,
};

type OpenUrlFn = Box<dyn FnMut(&str)>;
type RightClickPasteFn = Box<dyn FnMut()>;
type CopySelectionFn = Box<dyn Fn(&str) -> bool>;

/// Live chrome lines mounted as the TUI child (TS document container snapshot).
pub struct SharedLineView {
    pub lines: Rc<RefCell<Vec<String>>>,
}

impl Component for SharedLineView {
    fn render(&self, width: usize) -> Vec<String> {
        self.lines
            .borrow()
            .iter()
            .map(|line| {
                if pi_tui::visible_width_stripped(line) > width {
                    pi_tui::truncate_to_width_ansi(line, width, "…", false)
                } else {
                    line.clone()
                }
            })
            .collect()
    }

    fn invalidate(&mut self) {}
}

/// Full chrome for the main-screen renderer (document + dock stacked).
pub struct CombinedLineView {
    pub document: Rc<RefCell<Vec<String>>>,
    pub dock: Rc<RefCell<Vec<String>>>,
}

impl Component for CombinedLineView {
    fn render(&self, width: usize) -> Vec<String> {
        let mut lines = SharedLineView {
            lines: self.document.clone(),
        }
        .render(width);
        lines.extend(
            SharedLineView {
                lines: self.dock.clone(),
            }
            .render(width),
        );
        lines
    }

    fn invalidate(&mut self) {}
}

/// TS document ScrollView + editor/footer dock panes.
#[derive(Clone)]
pub struct ChromePanes {
    pub document: Rc<RefCell<Vec<String>>>,
    pub dock: Rc<RefCell<Vec<String>>>,
}

impl ChromePanes {
    pub fn new(document: Vec<String>, dock: Vec<String>) -> Self {
        Self {
            document: Rc::new(RefCell::new(document)),
            dock: Rc::new(RefCell::new(dock)),
        }
    }

    pub fn sync(&self, document: Vec<String>, dock: Vec<String>) {
        *self.document.borrow_mut() = document;
        *self.dock.borrow_mut() = dock;
    }
}

pub struct InteractiveTuiOptions {
    pub tui_mode: TuiMode,
    pub show_hardware_cursor: bool,
    pub log_directory: PathBuf,
    pub terminal: Box<dyn TerminalIo>,
    pub theme: Theme,
    pub copy_on_select: bool,
    pub open_url: Option<OpenUrlFn>,
    pub on_right_click_paste: Option<RightClickPasteFn>,
    pub copy_selection: Option<CopySelectionFn>,
}

impl InteractiveTuiOptions {
    pub fn with_process_terminal(
        tui_mode: TuiMode,
        theme: Theme,
        show_hardware_cursor: bool,
        log_directory: PathBuf,
        copy_on_select: bool,
    ) -> Self {
        Self {
            tui_mode,
            show_hardware_cursor,
            log_directory,
            terminal: Box::new(ProcessTerminal::live()),
            theme,
            copy_on_select,
            open_url: Some(Box::new(|url| {
                let _ = open_browser(url);
            })),
            on_right_click_paste: None,
            copy_selection: Some(Box::new(|text| {
                let _ = copy_text(text);
                true
            })),
        }
    }
}

/// TS `createInteractiveTui`.
pub enum InteractiveTui {
    Main(Box<TuiMainScreen>),
    Alt(Box<TuiAltScreen>),
}

impl InteractiveTui {
    pub fn mode(&self) -> TuiRuntimeMode {
        match self {
            Self::Main(tui) => tui.mode(),
            Self::Alt(tui) => tui.mode(),
        }
    }

    pub fn product_mode(&self) -> TuiMode {
        match self {
            Self::Main(_) => TuiMode::Regular,
            Self::Alt(_) => TuiMode::Fullscreen,
        }
    }

    pub fn is_viewport_tui(&self) -> bool {
        matches!(self, Self::Alt(_))
    }

    pub fn add_child(&mut self, component: Box<dyn Component>) {
        match self {
            Self::Main(tui) => tui.add_child(component),
            Self::Alt(tui) => tui.add_child(component),
        }
    }

    pub fn set_focus_child(&mut self, index: usize) {
        match self {
            Self::Main(tui) => tui.set_focus_child(index),
            Self::Alt(tui) => tui.set_focus_child(index),
        }
    }

    pub fn set_layout_root(&mut self, component: Box<dyn Component>) {
        if let Self::Alt(tui) = self {
            tui.set_layout_root(component);
        }
    }

    pub fn set_clear_on_shrink(&mut self, enabled: bool) {
        match self {
            Self::Main(tui) => tui.base.set_clear_on_shrink(enabled),
            Self::Alt(tui) => tui.base.set_clear_on_shrink(enabled),
        }
    }

    pub fn has_overlay_entries(&self) -> bool {
        match self {
            Self::Main(tui) => tui.base.has_overlay_entries(),
            Self::Alt(tui) => tui.base.has_overlay_entries(),
        }
    }

    pub fn hide_overlay(&mut self) {
        match self {
            Self::Main(tui) => tui.base.hide_overlay(),
            Self::Alt(tui) => tui.base.hide_overlay(),
        }
    }

    pub fn start(&mut self) {
        match self {
            Self::Main(tui) => tui.start(),
            Self::Alt(tui) => tui.start(),
        }
    }

    pub fn stop(&mut self, options: TuiStopOptions) {
        match self {
            Self::Main(tui) => tui.stop(options),
            Self::Alt(tui) => tui.stop(options),
        }
    }

    pub fn handle_input(&mut self, data: &str) {
        match self {
            Self::Main(tui) => tui.handle_input(data),
            Self::Alt(tui) => tui.handle_input(data),
        }
    }

    /// Alt-screen viewport/search/selection only. Main-screen always returns false
    /// so InteractiveSession keeps owning editor and slash keys.
    pub fn handle_host_input(&mut self, data: &str) -> bool {
        match self {
            Self::Main(_) => false,
            Self::Alt(tui) => tui.handle_host_input(data),
        }
    }

    pub fn render_now(&mut self, force: bool) {
        match self {
            Self::Main(tui) => tui.render_now(force),
            Self::Alt(tui) => tui.render_now(force),
        }
    }

    pub fn request_render(&mut self, force: bool) {
        match self {
            Self::Main(tui) => tui.request_render(force),
            Self::Alt(tui) => tui.request_render(force),
        }
    }

    pub fn invalidate(&mut self) {
        match self {
            Self::Main(tui) => tui.base.invalidate(),
            Self::Alt(tui) => tui.invalidate(),
        }
    }

    pub fn tick(&mut self, ms: u64) {
        match self {
            Self::Main(tui) => tui.base.tick(ms),
            Self::Alt(tui) => tui.tick(ms),
        }
    }

    pub fn flash(&mut self, message: &str) {
        if let Self::Alt(tui) = self {
            tui.flash(message, None);
        }
    }

    pub fn get_copy_on_select(&self) -> bool {
        match self {
            Self::Main(_) => false,
            Self::Alt(tui) => tui.get_copy_on_select(),
        }
    }

    pub fn has_active_selection(&self) -> bool {
        match self {
            Self::Main(_) => false,
            Self::Alt(tui) => tui.has_active_selection(),
        }
    }

    pub fn get_active_selection_text(&self) -> Option<String> {
        match self {
            Self::Main(_) => None,
            Self::Alt(tui) => tui.get_active_selection_text(),
        }
    }

    pub fn copy_active_selection_to_clipboard(&mut self) -> bool {
        match self {
            Self::Main(_) => false,
            Self::Alt(tui) => tui.copy_active_selection_to_clipboard(),
        }
    }

    pub fn show_overlay(
        &mut self,
        component: Box<dyn Component>,
        options: OverlayOptions,
    ) -> Option<OverlayHandle> {
        match self {
            Self::Main(tui) => Some(tui.base.show_overlay(component, options)),
            Self::Alt(tui) => Some(tui.show_overlay(component, options)),
        }
    }

    pub fn capture_main_render_state(&self) -> Option<TuiMainScreenRenderState> {
        match self {
            Self::Main(tui) => Some(tui.capture_render_state()),
            Self::Alt(_) => None,
        }
    }

    pub fn set_terminal_size(&mut self, columns: usize, rows: usize) {
        let terminal = match self {
            Self::Main(tui) => &mut tui.base.terminal,
            Self::Alt(tui) => &mut tui.base.terminal,
        };
        if let Some(process) = (*terminal).as_any_mut().downcast_mut::<ProcessTerminal>() {
            process.set_size(columns, rows);
        }
        if let Some(memory) = (*terminal).as_any_mut().downcast_mut::<MemoryTerminal>() {
            memory.columns = columns.max(1);
            memory.rows = rows.max(1);
        }
    }

    pub fn set_progress(&mut self, active: bool) {
        match self {
            Self::Main(tui) => tui.base.terminal.set_progress(active),
            Self::Alt(tui) => tui.base.terminal.set_progress(active),
        }
    }

    pub fn take_terminal(self) -> Box<dyn TerminalIo> {
        match self {
            Self::Main(tui) => (*tui).take_terminal(),
            Self::Alt(tui) => (*tui).take_terminal(),
        }
    }
}

pub fn create_interactive_tui(options: InteractiveTuiOptions) -> InteractiveTui {
    match options.tui_mode {
        TuiMode::Fullscreen => {
            let theme = options.theme;
            let search_match = {
                let theme = theme.clone();
                Rc::new(move |text: &str| {
                    theme.underline(&theme.bg("searchMatchBg", &theme.fg("searchMatchText", text)))
                })
            };
            let search_current =
                {
                    let theme = theme.clone();
                    Rc::new(move |text: &str| {
                        theme.bold(&theme.inverse(
                            &theme.bg("searchMatchBg", &theme.fg("searchMatchText", text)),
                        ))
                    })
                };
            InteractiveTui::Alt(Box::new(TuiAltScreen::with_options(
                options.terminal,
                TuiAltScreenOptions {
                    copy_on_select: options.copy_on_select,
                    show_hardware_cursor: Some(options.show_hardware_cursor),
                    log_directory: Some(options.log_directory),
                    search_match_style: Some(search_match),
                    search_current_match_style: Some(search_current),
                    open_url: options.open_url,
                    on_right_click_paste: options.on_right_click_paste,
                    copy_selection: options.copy_selection,
                    ..TuiAltScreenOptions::default()
                },
            )))
        }
        TuiMode::Regular => InteractiveTui::Main(Box::new(TuiMainScreen::with_options(
            options.terminal,
            Some(options.show_hardware_cursor),
            Some(options.log_directory),
        ))),
    }
}

/// Recreate the renderer when `/settings` switches `tuiMode`.
/// Returns `(tui, switched)` — `switched` is false when overlays block the change (TS).
pub fn switch_tui_mode(
    current: InteractiveTui,
    mode: TuiMode,
    mut options: InteractiveTuiOptions,
    start_renderer: bool,
) -> (InteractiveTui, bool) {
    if current.product_mode() == mode {
        return (current, true);
    }
    if current.has_overlay_entries() {
        return (current, false);
    }
    let state = current.capture_main_render_state();
    let mut current = current;
    current.stop(TuiStopOptions {
        preserve_screen: true,
    });
    options.terminal = current.take_terminal();
    options.tui_mode = mode;
    let mut next = create_interactive_tui(options);
    if let (Some(state), InteractiveTui::Main(main)) = (state, &mut next) {
        main.restore_render_state(state);
    }
    if start_renderer {
        next.start();
    }
    (next, true)
}

/// TS `stopInteractiveTui`.
pub fn stop_interactive_tui(
    mut tui: InteractiveTui,
    fullscreen_exit_output: &str,
    options: InteractiveTuiOptions,
    remount: impl FnOnce(&mut InteractiveTui),
) {
    if tui.is_viewport_tui() && fullscreen_exit_output != "resume-hint" {
        while tui.has_overlay_entries() {
            tui.hide_overlay();
        }
        let (mut next, _) = switch_tui_mode(tui, TuiMode::Regular, options, false);
        remount(&mut next);
        next.render_now(false);
        tui = next;
    }
    let preserve_screen = tui.is_viewport_tui();
    tui.stop(TuiStopOptions { preserve_screen });
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyCommandResult {
    CopiedSelection,
    CopiedAssistant,
    NoAssistant,
    Failed(String),
}

/// TS `InteractiveMode.handleCopyCommand`.
pub fn handle_copy_command(
    tui: &mut InteractiveTui,
    last_assistant_text: Option<&str>,
    flash_confirmation: bool,
    prefer_selection: bool,
) -> CopyCommandResult {
    if prefer_selection
        && tui.is_viewport_tui()
        && !tui.get_copy_on_select()
        && tui.has_active_selection()
    {
        if tui.copy_active_selection_to_clipboard() {
            return CopyCommandResult::CopiedSelection;
        }
        return CopyCommandResult::Failed("Copy failed".into());
    }
    let Some(text) = last_assistant_text.filter(|value| !value.is_empty()) else {
        return CopyCommandResult::NoAssistant;
    };
    let _ = copy_text(text);
    if flash_confirmation && tui.is_viewport_tui() {
        tui.flash("Copied!");
    }
    CopyCommandResult::CopiedAssistant
}

pub fn remount_chrome(tui: &mut InteractiveTui, lines: Rc<RefCell<Vec<String>>>) {
    tui.add_child(Box::new(SharedLineView { lines }));
    tui.set_focus_child(0);
}

pub fn remount_chrome_panes(tui: &mut InteractiveTui, panes: &ChromePanes) {
    if tui.is_viewport_tui() {
        let scroll = ScrollView::new(
            Box::new(SharedLineView {
                lines: panes.document.clone(),
            }),
            ScrollViewOptions {
                follow: ScrollFollow::End,
                primary: true,
                ..ScrollViewOptions::default()
            },
        )
        .expect("scroll view");
        let mut root = VStack::new(0);
        root.add_child(
            Box::new(scroll),
            StackEntryOptions {
                basis: Some(StackBasis::Fixed(0)),
                grow: Some(1),
                min_size: Some(1),
                ..StackEntryOptions::default()
            },
        );
        root.add_child(
            Box::new(SharedLineView {
                lines: panes.dock.clone(),
            }),
            StackEntryOptions {
                basis: Some(StackBasis::Auto),
                min_size: Some(1),
                ..StackEntryOptions::default()
            },
        );
        tui.set_layout_root(Box::new(root));
        tui.add_child(Box::new(SharedLineView {
            lines: panes.document.clone(),
        }));
    } else {
        tui.add_child(Box::new(CombinedLineView {
            document: panes.document.clone(),
            dock: panes.dock.clone(),
        }));
    }
    tui.set_focus_child(0);
}

pub fn sync_chrome_lines(lines: &Rc<RefCell<Vec<String>>>, rendered: Vec<String>) {
    *lines.borrow_mut() = rendered;
}

#[cfg(test)]
mod tests {
    use super::*;
    use pi_tui::TuiText;

    fn memory_options(mode: TuiMode, columns: usize, rows: usize) -> InteractiveTuiOptions {
        InteractiveTuiOptions {
            tui_mode: mode,
            show_hardware_cursor: false,
            log_directory: PathBuf::from("/tmp"),
            terminal: Box::new(MemoryTerminal::new(columns, rows)),
            theme: Theme::default(),
            copy_on_select: true,
            open_url: None,
            on_right_click_paste: None,
            copy_selection: None,
        }
    }

    #[test]
    fn selects_alt_screen_only_when_fullscreen() {
        let main = create_interactive_tui(memory_options(TuiMode::Regular, 40, 8));
        assert_eq!(main.mode(), TuiRuntimeMode::Regular);
        assert!(!main.is_viewport_tui());

        let mut alt = create_interactive_tui(memory_options(TuiMode::Fullscreen, 40, 8));
        assert_eq!(alt.mode(), TuiRuntimeMode::Fullscreen);
        assert!(alt.is_viewport_tui());
        alt.add_child(Box::new(TuiText::new("hello", 0, 0)));
        alt.start();
        let output = match &alt {
            InteractiveTui::Alt(tui) => (*tui.base.terminal)
                .as_any()
                .downcast_ref::<MemoryTerminal>()
                .expect("memory")
                .output(),
            InteractiveTui::Main(_) => String::new(),
        };
        assert!(output.contains("\x1b[?1049h"));
        alt.stop(TuiStopOptions::default());
    }

    #[test]
    fn switch_mode_reuses_terminal_and_preserves_screen() {
        let mut tui = create_interactive_tui(memory_options(TuiMode::Regular, 40, 8));
        tui.add_child(Box::new(TuiText::new("content", 0, 0)));
        tui.start();
        let (next, ok) = switch_tui_mode(
            tui,
            TuiMode::Fullscreen,
            memory_options(TuiMode::Fullscreen, 40, 8),
            true,
        );
        tui = next;
        assert!(ok);
        assert!(tui.is_viewport_tui());
        tui.stop(TuiStopOptions {
            preserve_screen: true,
        });
    }

    #[test]
    fn host_input_consumes_alt_screen_search() {
        let mut tui = create_interactive_tui(memory_options(TuiMode::Fullscreen, 40, 8));
        tui.add_child(Box::new(TuiText::new("alpha beta", 0, 0)));
        tui.start();
        assert!(tui.handle_host_input("\x1b[102;6u"));
        assert!(tui.has_overlay_entries());
        tui.stop(TuiStopOptions::default());
    }

    #[test]
    fn host_input_passes_through_on_main_screen() {
        let mut tui = create_interactive_tui(memory_options(TuiMode::Regular, 40, 8));
        tui.add_child(Box::new(TuiText::new("alpha", 0, 0)));
        tui.start();
        assert!(!tui.handle_host_input("a"));
        tui.stop(TuiStopOptions::default());
    }

    #[test]
    fn resume_hint_exit_keeps_fullscreen_preserve() {
        let mut tui = create_interactive_tui(memory_options(TuiMode::Fullscreen, 40, 8));
        let lines = Rc::new(RefCell::new(vec!["content".into()]));
        remount_chrome(&mut tui, lines.clone());
        tui.start();
        stop_interactive_tui(
            tui,
            "resume-hint",
            memory_options(TuiMode::Fullscreen, 40, 8),
            |next| remount_chrome(next, lines.clone()),
        );
    }

    #[test]
    fn transcript_exit_rewrites_on_main_screen() {
        let mut tui = create_interactive_tui(memory_options(TuiMode::Fullscreen, 40, 8));
        let lines = Rc::new(RefCell::new(vec!["line one".into(), "line two".into()]));
        remount_chrome(&mut tui, lines.clone());
        tui.start();
        stop_interactive_tui(
            tui,
            "transcript",
            memory_options(TuiMode::Regular, 40, 8),
            |next| remount_chrome(next, lines.clone()),
        );
    }

    #[test]
    fn copy_command_flashes_on_fullscreen_and_status_on_regular() {
        std::env::set_var("PI_COPY_DRY_RUN", "1");
        let mut alt = create_interactive_tui(memory_options(TuiMode::Fullscreen, 40, 4));
        alt.start();
        assert_eq!(
            handle_copy_command(&mut alt, Some("assistant response"), true, true),
            CopyCommandResult::CopiedAssistant
        );
        alt.stop(TuiStopOptions::default());

        let mut main = create_interactive_tui(memory_options(TuiMode::Regular, 40, 4));
        assert_eq!(
            handle_copy_command(&mut main, Some("assistant response"), true, true),
            CopyCommandResult::CopiedAssistant
        );
        assert_eq!(
            handle_copy_command(&mut main, None, true, true),
            CopyCommandResult::NoAssistant
        );
        std::env::remove_var("PI_COPY_DRY_RUN");
    }

    #[test]
    fn shared_line_view_tracks_session_chrome() {
        let lines = Rc::new(RefCell::new(vec!["hello".into()]));
        let view = SharedLineView {
            lines: lines.clone(),
        };
        assert_eq!(view.render(40), vec!["hello".to_string()]);
        sync_chrome_lines(&lines, vec!["updated".into()]);
        assert_eq!(view.render(40), vec!["updated".to_string()]);
    }

    #[test]
    fn fullscreen_layout_keeps_dock_visible() {
        let mut tui = create_interactive_tui(memory_options(TuiMode::Fullscreen, 20, 6));
        let panes = ChromePanes::new(
            (1..=8).map(|index| format!("line {index}")).collect(),
            vec!["editor".into(), "footer".into()],
        );
        remount_chrome_panes(&mut tui, &panes);
        tui.start();
        let view = match &tui {
            InteractiveTui::Alt(inner) => inner.viewport_lines(),
            InteractiveTui::Main(_) => Vec::new(),
        };
        assert_eq!(
            view,
            vec!["line 5", "line 6", "line 7", "line 8", "editor", "footer"]
        );
        tui.stop(TuiStopOptions::default());
    }
}
