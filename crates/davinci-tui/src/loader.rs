//! Loader / CancellableLoader matching `vendor/pi/packages/tui/src/components/loader.ts`.

use crate::keybindings::Keybindings;
use crate::render::Component;
use crate::tui_text::TuiText;

pub const DEFAULT_LOADER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
pub const DEFAULT_LOADER_INTERVAL_MS: u64 = 80;

#[derive(Debug, Clone)]
pub struct LoaderIndicatorOptions {
    pub frames: Option<Vec<String>>,
    pub interval_ms: Option<u64>,
}

pub struct Loader {
    text: TuiText,
    frames: Vec<String>,
    interval_ms: u64,
    current_frame: usize,
    elapsed_ms: u64,
    running: bool,
    render_indicator_verbatim: bool,
    spinner_color: Box<dyn Fn(&str) -> String>,
    message_color: Box<dyn Fn(&str) -> String>,
    message: String,
}

impl Loader {
    pub fn new(
        spinner_color: impl Fn(&str) -> String + 'static,
        message_color: impl Fn(&str) -> String + 'static,
        message: impl Into<String>,
        indicator: Option<LoaderIndicatorOptions>,
    ) -> Self {
        let mut loader = Self {
            text: TuiText::new("", 1, 0),
            frames: DEFAULT_LOADER_FRAMES
                .iter()
                .map(|frame| (*frame).to_string())
                .collect(),
            interval_ms: DEFAULT_LOADER_INTERVAL_MS,
            current_frame: 0,
            elapsed_ms: 0,
            running: false,
            render_indicator_verbatim: false,
            spinner_color: Box::new(spinner_color),
            message_color: Box::new(message_color),
            message: message.into(),
        };
        loader.set_indicator(indicator);
        loader
    }

    pub fn start(&mut self) {
        self.update_display();
        self.restart_animation();
    }

    pub fn stop(&mut self) {
        self.running = false;
        self.elapsed_ms = 0;
    }

    pub fn set_message(&mut self, message: impl Into<String>) {
        self.message = message.into();
        self.update_display();
    }

    pub fn set_indicator(&mut self, indicator: Option<LoaderIndicatorOptions>) {
        self.render_indicator_verbatim = indicator.is_some();
        match indicator {
            Some(options) => {
                self.frames = options.frames.unwrap_or_else(|| {
                    DEFAULT_LOADER_FRAMES
                        .iter()
                        .map(|frame| (*frame).to_string())
                        .collect()
                });
                self.interval_ms = options
                    .interval_ms
                    .filter(|ms| *ms > 0)
                    .unwrap_or(DEFAULT_LOADER_INTERVAL_MS);
            }
            None => {
                self.frames = DEFAULT_LOADER_FRAMES
                    .iter()
                    .map(|frame| (*frame).to_string())
                    .collect();
                self.interval_ms = DEFAULT_LOADER_INTERVAL_MS;
            }
        }
        self.current_frame = 0;
        self.start();
    }

    pub fn tick(&mut self, ms: u64) {
        if !self.running || self.frames.len() <= 1 {
            return;
        }
        self.elapsed_ms += ms;
        while self.elapsed_ms >= self.interval_ms {
            self.elapsed_ms -= self.interval_ms;
            self.current_frame = (self.current_frame + 1) % self.frames.len();
            self.update_display();
        }
    }

    fn restart_animation(&mut self) {
        self.stop();
        if self.frames.len() <= 1 {
            self.update_display();
            return;
        }
        self.running = true;
        self.update_display();
    }

    fn update_display(&mut self) {
        let frame = self
            .frames
            .get(self.current_frame)
            .cloned()
            .unwrap_or_default();
        let rendered_frame = if self.render_indicator_verbatim {
            frame.clone()
        } else {
            (self.spinner_color)(&frame)
        };
        let indicator = if frame.is_empty() {
            String::new()
        } else {
            format!("{rendered_frame} ")
        };
        self.text.set_text(format!(
            "{indicator}{}",
            (self.message_color)(&self.message)
        ));
    }
}

impl Component for Loader {
    fn render(&self, width: usize) -> Vec<String> {
        let mut lines = vec![String::new()];
        lines.extend(self.text.render(width));
        lines
    }

    fn invalidate(&mut self) {
        self.text.invalidate();
    }
}

pub struct CancellableLoader {
    loader: Loader,
    aborted: bool,
    on_abort: Option<Box<dyn FnMut()>>,
}

impl CancellableLoader {
    pub fn new(
        spinner_color: impl Fn(&str) -> String + 'static,
        message_color: impl Fn(&str) -> String + 'static,
        message: impl Into<String>,
        indicator: Option<LoaderIndicatorOptions>,
    ) -> Self {
        Self {
            loader: Loader::new(spinner_color, message_color, message, indicator),
            aborted: false,
            on_abort: None,
        }
    }

    pub fn set_on_abort(&mut self, on_abort: impl FnMut() + 'static) {
        self.on_abort = Some(Box::new(on_abort));
    }

    pub fn aborted(&self) -> bool {
        self.aborted
    }

    pub fn tick(&mut self, ms: u64) {
        self.loader.tick(ms);
    }

    pub fn stop(&mut self) {
        self.loader.stop();
    }

    pub fn dispose(&mut self) {
        self.loader.stop();
    }
}

impl Component for CancellableLoader {
    fn render(&self, width: usize) -> Vec<String> {
        self.loader.render(width)
    }

    fn handle_input(&mut self, data: &str) {
        if Keybindings::defaults().matches(data, "tui.select.cancel") {
            self.aborted = true;
            if let Some(on_abort) = &mut self.on_abort {
                on_abort();
            }
        }
    }

    fn invalidate(&mut self) {
        self.loader.invalidate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keybindings::key_to_bytes;

    fn identity(value: &str) -> String {
        value.to_string()
    }

    #[test]
    fn loader_ticks_frames_and_hides_empty_indicator() {
        let mut loader = Loader::new(
            identity,
            identity,
            "Working...",
            Some(LoaderIndicatorOptions {
                frames: Some(vec!["A".into(), "B".into()]),
                interval_ms: Some(10),
            }),
        );
        let first = loader.render(20);
        assert_eq!(first[0], "");
        assert!(first[1].contains("A Working..."));
        loader.tick(10);
        assert!(loader.render(20)[1].contains("B Working..."));
        loader.set_indicator(Some(LoaderIndicatorOptions {
            frames: Some(Vec::new()),
            interval_ms: Some(10),
        }));
        assert!(loader.render(20)[1].contains("Working..."));
        assert!(!loader.render(20)[1].contains("A "));
    }

    #[test]
    fn cancellable_loader_aborts_on_escape() {
        let mut loader = CancellableLoader::new(identity, identity, "Working...", None);
        loader.handle_input(&key_to_bytes("escape"));
        assert!(loader.aborted());
    }
}
