//! Structured decision dialog: 1-4 questions answered in one submission.
//!
//! Upstream reference: vendor/davinci (clarification and structured user decisions).
//! Exclusive input ownership prevents accidental grants or unverified execution:
//! every key is consumed here while the dialog is open, nothing is submitted
//! until Enter on the last page, and Esc or `d` end the whole dialog without
//! sending a partial answer.
//!
//! Pages: one per question, then a review page when there is more than one
//! question. Answers live in each question's state, so moving between pages
//! never loses a selection or a half-typed custom answer.

use crate::davinci::theme::Theme;
use crate::davinci::ui::{
    clip_ellipsis, hint_row, section_detail, section_row, span, truncate_run, Surface,
};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Matches the runtime's bound on one custom answer.
pub const MAX_CUSTOM_BYTES: usize = 8_192;

pub fn commits_decision(event: &str, answer_valid: bool) -> bool {
    event == "enter" && answer_valid
}

/// What a key does while the dialog is open. Nothing is ever delegated, so
/// shift+tab cannot reach the permission-mode cycle behind the dialog.
pub fn decision_key(open: bool, key: &str, is_custom_mode: bool) -> &'static str {
    if !open {
        return "delegate";
    }
    match key {
        "shift_tab" => "previous_question",
        "tab" => "next_question",
        "escape" => "cancel",
        "enter" => "confirm",
        "up" => "previous",
        "down" => "next",
        "space" if !is_custom_mode => "toggle",
        "left" | "right" if is_custom_mode => "edit",
        "left" => "previous_question",
        "right" => "next_question",
        "ctrl_d" => "defer",
        "d" if !is_custom_mode => "defer",
        "i" if !is_custom_mode => "inspect",
        "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" if !is_custom_mode => "select_number",
        _ => {
            if is_custom_mode {
                "edit"
            } else {
                "consume"
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionModalOption {
    pub id: String,
    pub label: String,
    pub explanation: String,
    pub recommended: bool,
}

/// One question's answer as submitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestionAnswer {
    /// The single option picked on a single-select question.
    Choice(String),
    /// Free text replacing the options of a single-select question.
    Custom(String),
    /// Every checked option of a multi-select question, plus the custom
    /// answer when one was typed. The custom text counts as one selection.
    Choices {
        choice_ids: Vec<String>,
        custom_text: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionResult {
    /// One answer per question, in the order the questions were asked.
    Answers(Vec<(String, QuestionAnswer)>),
    Defer,
    Cancel,
}

/// One question and everything the user has done to it so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionModalQuestion {
    pub id: String,
    pub title: String,
    pub question: String,
    pub materiality: String,
    pub evidence_refs: Vec<String>,
    pub options: Vec<DecisionModalOption>,
    pub allow_custom: bool,
    pub custom_only: bool,
    pub multi_select: bool,
    pub min_selections: usize,
    pub max_selections: usize,
    /// Focused row: an option index, or `options.len()` for the custom row.
    pub cursor: usize,
    /// Single-select: the picked row, which may be the custom row.
    pub chosen: Option<usize>,
    /// Multi-select: one flag per option.
    pub checked: Vec<bool>,
    pub custom_text: String,
    /// Byte offset into `custom_text`, always on a char boundary.
    pub custom_cursor: usize,
}

impl DecisionModalQuestion {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        question: impl Into<String>,
        materiality: impl Into<String>,
        evidence_refs: Vec<String>,
        options: Vec<DecisionModalOption>,
        allow_custom: bool,
        custom_only: bool,
    ) -> Self {
        let allow_custom = allow_custom || custom_only;
        let cursor = if custom_only || options.is_empty() {
            options.len()
        } else {
            0
        };
        Self {
            id: id.into(),
            title: title.into(),
            question: question.into(),
            materiality: materiality.into(),
            evidence_refs,
            checked: vec![false; options.len()],
            options,
            allow_custom,
            custom_only,
            multi_select: false,
            min_selections: 1,
            max_selections: 1,
            cursor,
            chosen: None,
            custom_text: String::new(),
            custom_cursor: 0,
        }
    }

    /// Make this a multi-select question answered with `min..=max` picks.
    pub fn multi(mut self, min: usize, max: usize) -> Self {
        self.multi_select = true;
        self.min_selections = min;
        self.max_selections = max;
        self
    }

    fn has_custom_row(&self) -> bool {
        self.allow_custom
    }

    fn row_count(&self) -> usize {
        if self.custom_only {
            return 1;
        }
        self.options.len() + usize::from(self.has_custom_row())
    }

    fn first_row(&self) -> usize {
        if self.custom_only {
            self.options.len()
        } else {
            0
        }
    }

    /// Whether keys go to the custom-answer text field.
    pub fn on_custom(&self) -> bool {
        self.has_custom_row() && self.cursor == self.options.len()
    }

    fn custom(&self) -> Option<String> {
        let text = self.custom_text.trim();
        (!text.is_empty()).then(|| text.to_string())
    }

    fn selection_count(&self) -> usize {
        self.checked.iter().filter(|c| **c).count() + usize::from(self.custom().is_some())
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let first = self.first_row() as isize;
        let last = first + self.row_count() as isize - 1;
        self.cursor = (self.cursor as isize + delta).clamp(first, last.max(first)) as usize;
    }

    /// Space on an option: check or uncheck it, or pick it on single-select.
    pub fn toggle(&mut self) -> Result<(), String> {
        if self.cursor >= self.options.len() {
            return Ok(());
        }
        if !self.multi_select {
            self.chosen = Some(self.cursor);
            return Ok(());
        }
        let index = self.cursor;
        if !self.checked[index] && self.selection_count() >= self.max_selections {
            return Err(format!(
                "Choose at most {}; uncheck one first",
                self.max_selections
            ));
        }
        self.checked[index] = !self.checked[index];
        Ok(())
    }

    /// Insert at the text cursor. Refused past the runtime's limit, so the
    /// dialog never builds an answer the agent would reject.
    pub fn insert_char(&mut self, c: char) -> Result<(), String> {
        if self.custom_text.len() + c.len_utf8() > MAX_CUSTOM_BYTES {
            return Err(format!(
                "A custom answer is limited to {MAX_CUSTOM_BYTES} bytes"
            ));
        }
        self.custom_text.insert(self.custom_cursor, c);
        self.custom_cursor += c.len_utf8();
        self.sync_custom_choice();
        Ok(())
    }

    /// On single-select, typing an answer is choosing it, and clearing the
    /// text unchooses it. Every exit then agrees on what was picked.
    fn sync_custom_choice(&mut self) {
        if self.multi_select || self.custom_only {
            return;
        }
        let custom_row = self.options.len();
        if self.custom().is_some() {
            self.chosen = Some(custom_row);
        } else if self.chosen == Some(custom_row) {
            self.chosen = None;
        }
    }

    pub fn backspace(&mut self) {
        if let Some((prev, _)) = self.custom_text[..self.custom_cursor].char_indices().last() {
            self.custom_text.drain(prev..self.custom_cursor);
            self.custom_cursor = prev;
            self.sync_custom_choice();
        }
    }

    pub fn move_text_cursor(&mut self, delta: isize) {
        if delta < 0 {
            if let Some((prev, _)) = self.custom_text[..self.custom_cursor].char_indices().last() {
                self.custom_cursor = prev;
            }
        } else if let Some(c) = self.custom_text[self.custom_cursor..].chars().next() {
            self.custom_cursor += c.len_utf8();
        }
    }

    /// Enter on this question's page. Single-select picks the focused row;
    /// multi-select keeps what is checked. Either way the result must be a
    /// valid answer before the dialog moves on.
    fn confirm_focus(&mut self) -> Result<(), String> {
        if !self.multi_select {
            if self.on_custom() {
                if self.custom().is_none() {
                    return Err("Type an answer, or move up to pick an option".into());
                }
                self.chosen = Some(self.options.len());
            } else if self.cursor < self.options.len() {
                self.chosen = Some(self.cursor);
            }
        }
        self.answer().map(|_| ())
    }

    /// This question's answer, or why it cannot be submitted yet.
    pub fn answer(&self) -> Result<QuestionAnswer, String> {
        if self.custom_only {
            return self
                .custom()
                .map(QuestionAnswer::Custom)
                .ok_or_else(|| "Type an answer".to_string());
        }
        if !self.multi_select {
            return match self.chosen {
                Some(index) if index < self.options.len() => {
                    Ok(QuestionAnswer::Choice(self.options[index].id.clone()))
                }
                Some(_) => self
                    .custom()
                    .map(QuestionAnswer::Custom)
                    .ok_or_else(|| "The custom answer is empty".to_string()),
                None => Err("Pick an answer".into()),
            };
        }
        let count = self.selection_count();
        if count < self.min_selections {
            return Err(format!(
                "Choose at least {} (have {count})",
                self.min_selections
            ));
        }
        if count > self.max_selections {
            return Err(format!(
                "Choose at most {} (have {count})",
                self.max_selections
            ));
        }
        Ok(QuestionAnswer::Choices {
            choice_ids: self
                .options
                .iter()
                .zip(&self.checked)
                .filter(|(_, checked)| **checked)
                .map(|(option, _)| option.id.clone())
                .collect(),
            custom_text: self.custom(),
        })
    }

    /// The answer in words, for the review page and the progress strip.
    pub fn summary(&self) -> Option<String> {
        let label = |id: &str| {
            self.options
                .iter()
                .find(|option| option.id == id)
                .map(|option| option.label.clone())
                .unwrap_or_else(|| id.to_string())
        };
        match self.answer().ok()? {
            QuestionAnswer::Choice(id) => Some(label(&id)),
            QuestionAnswer::Custom(text) => Some(format!("\"{text}\"")),
            QuestionAnswer::Choices {
                choice_ids,
                custom_text,
            } => {
                let mut parts: Vec<String> = choice_ids.iter().map(|id| label(id)).collect();
                if let Some(text) = custom_text {
                    parts.push(format!("\"{text}\""));
                }
                Some(if parts.is_empty() {
                    "none".into()
                } else {
                    parts.join(", ")
                })
            }
        }
    }

    fn selection_rule(&self) -> Option<String> {
        if !self.multi_select {
            return None;
        }
        let (min, max) = (self.min_selections, self.max_selections);
        Some(match (min, max) {
            (0, max) => format!("Choose any, up to {max}"),
            (min, max) if min == max => format!("Choose exactly {min}"),
            (min, max) => format!("Choose {min} to {max}"),
        })
    }
}

/// What a key did to the dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalFlow {
    /// Still open.
    Open,
    /// Submitted, deferred or cancelled: `outcome` is set.
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionModalState {
    pub request_id: String,
    pub plan_revision: u64,
    pub questions: Vec<DecisionModalQuestion>,
    /// `0..questions.len()` is a question; `questions.len()` is the review
    /// page, which exists only when there is more than one question.
    pub page: usize,
    pub inspecting_evidence: bool,
    /// The last validation failure, shown until the next edit.
    pub error: Option<String>,
    pub submitted: bool,
    pub outcome: Option<DecisionResult>,
}

impl DecisionModalState {
    pub fn new(
        request_id: impl Into<String>,
        plan_revision: u64,
        questions: Vec<DecisionModalQuestion>,
    ) -> Self {
        Self {
            request_id: request_id.into(),
            plan_revision,
            questions,
            page: 0,
            inspecting_evidence: false,
            error: None,
            submitted: false,
            outcome: None,
        }
    }

    fn last_page(&self) -> usize {
        if self.questions.len() > 1 {
            self.questions.len()
        } else {
            0
        }
    }

    pub fn on_review(&self) -> bool {
        self.questions.len() > 1 && self.page == self.questions.len()
    }

    /// The question on screen, if this is not the review page.
    pub fn current(&self) -> Option<&DecisionModalQuestion> {
        self.questions.get(self.page)
    }

    fn current_mut(&mut self) -> Option<&mut DecisionModalQuestion> {
        self.questions.get_mut(self.page)
    }

    /// Whether typed characters go to a custom-answer field.
    pub fn editing_text(&self) -> bool {
        self.current().is_some_and(DecisionModalQuestion::on_custom)
    }

    pub fn go_to_page(&mut self, page: usize) {
        self.page = page.min(self.last_page());
        self.error = None;
    }

    pub fn next_page(&mut self) {
        self.go_to_page(self.page.saturating_add(1));
    }

    pub fn previous_page(&mut self) {
        self.go_to_page(self.page.saturating_sub(1));
    }

    pub fn move_selection(&mut self, delta: isize) {
        self.error = None;
        if let Some(question) = self.current_mut() {
            question.move_cursor(delta);
        }
    }

    /// Digits focus an option; on multi-select they also toggle it. They
    /// never submit.
    pub fn select_number(&mut self, number: usize) {
        self.error = None;
        let Some(question) = self.current_mut() else {
            return;
        };
        if number == 0 || number > question.options.len() || question.custom_only {
            return;
        }
        question.cursor = number - 1;
        if let Err(error) = question.toggle() {
            self.error = Some(error);
        }
    }

    pub fn toggle(&mut self) {
        self.error = None;
        if let Some(Err(error)) = self.current_mut().map(DecisionModalQuestion::toggle) {
            self.error = Some(error);
        }
    }

    pub fn insert_char(&mut self, c: char) {
        self.error = None;
        if let Some(Err(error)) = self.current_mut().map(|q| q.insert_char(c)) {
            self.error = Some(error);
        }
    }

    /// Pasted text goes into the custom answer being typed, on one line.
    /// Anywhere else in the dialog it is dropped, never sent to the composer
    /// behind it.
    pub fn paste(&mut self, text: &str) {
        if self.submitted || !self.editing_text() {
            return;
        }
        for c in text.chars() {
            let c = if c.is_control() { ' ' } else { c };
            self.insert_char(c);
            if self.error.is_some() {
                break;
            }
        }
    }

    pub fn backspace(&mut self) {
        self.error = None;
        if let Some(question) = self.current_mut() {
            question.backspace();
        }
    }

    pub fn toggle_inspect(&mut self) {
        self.inspecting_evidence = !self.inspecting_evidence;
    }

    pub fn is_valid(&self) -> bool {
        !self.submitted && self.questions.iter().all(|q| q.answer().is_ok())
    }

    /// Enter: confirm this page and move on; on the last page, submit.
    pub fn confirm(&mut self) -> Option<DecisionResult> {
        if self.submitted {
            return None;
        }
        if let Some(question) = self.current_mut() {
            if let Err(error) = question.confirm_focus() {
                self.error = Some(error);
                return None;
            }
            if self.page < self.last_page() {
                self.next_page();
                return None;
            }
        }
        self.commit()
    }

    /// Submit every answer at once, or jump to the first unanswered
    /// question and say why. Nothing partial is ever sent.
    pub fn commit(&mut self) -> Option<DecisionResult> {
        if self.submitted {
            return None;
        }
        let mut answers = Vec::with_capacity(self.questions.len());
        for (index, question) in self.questions.iter().enumerate() {
            match question.answer() {
                Ok(answer) => answers.push((question.id.clone(), answer)),
                Err(error) => {
                    self.page = index;
                    self.error = Some(if self.questions.len() > 1 {
                        format!("{}: {error}", question.title)
                    } else {
                        error
                    });
                    return None;
                }
            }
        }
        self.finish(DecisionResult::Answers(answers))
    }

    pub fn defer(&mut self) -> Option<DecisionResult> {
        self.finish(DecisionResult::Defer)
    }

    pub fn cancel(&mut self) -> Option<DecisionResult> {
        self.finish(DecisionResult::Cancel)
    }

    fn finish(&mut self, result: DecisionResult) -> Option<DecisionResult> {
        if self.submitted {
            return None;
        }
        self.submitted = true;
        self.outcome = Some(result.clone());
        Some(result)
    }

    /// Route one key. Every key is consumed while the dialog is open.
    pub fn handle_key(&mut self, key: KeyEvent) -> ModalFlow {
        if key.kind == KeyEventKind::Release || self.submitted {
            return self.flow();
        }
        // AltGr arrives as Ctrl+Alt on Windows; those keys are characters.
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT);
        let editing = self.editing_text();
        match key.code {
            KeyCode::Esc => {
                self.cancel();
            }
            KeyCode::BackTab => self.previous_page(),
            KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => self.previous_page(),
            KeyCode::Tab => self.next_page(),
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Down => self.move_selection(1),
            KeyCode::Left if editing => {
                if let Some(question) = self.current_mut() {
                    question.move_text_cursor(-1);
                }
            }
            KeyCode::Right if editing => {
                if let Some(question) = self.current_mut() {
                    question.move_text_cursor(1);
                }
            }
            KeyCode::Left => self.previous_page(),
            KeyCode::Right => self.next_page(),
            KeyCode::Enter if key.kind == KeyEventKind::Press && !ctrl => {
                self.confirm();
            }
            KeyCode::Char('d') if ctrl => {
                self.defer();
            }
            KeyCode::Backspace if editing => self.backspace(),
            KeyCode::Char(c) if editing && !ctrl => self.insert_char(c),
            KeyCode::Char(' ') => self.toggle(),
            KeyCode::Char('d') => {
                self.defer();
            }
            KeyCode::Char('i') => self.toggle_inspect(),
            KeyCode::Char(digit @ '1'..='9') if key.modifiers.is_empty() => {
                self.select_number((digit as u8 - b'0') as usize)
            }
            _ => {}
        }
        self.flow()
    }

    fn flow(&self) -> ModalFlow {
        if self.submitted {
            ModalFlow::Closed
        } else {
            ModalFlow::Open
        }
    }
}

/// `✓ Title` for answered questions, `· Title` for open ones, the current
/// page highlighted; collapses to plain progress when it does not fit.
fn progress_strip(state: &DecisionModalState, width: u16, th: &Theme) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (index, question) in state.questions.iter().enumerate() {
        if index > 0 {
            spans.push(span("  ", th.muted));
        }
        let mark = if question.answer().is_ok() {
            "✓"
        } else {
            "·"
        };
        let color = if index == state.page {
            th.primary
        } else {
            th.muted
        };
        spans.push(span(
            format!("{mark} {}", clip_ellipsis(&question.title, 18)),
            color,
        ));
    }
    spans.push(span("  ", th.muted));
    spans.push(span(
        "Submit",
        if state.on_review() {
            th.primary
        } else {
            th.muted
        },
    ));
    let mut row = vec![span("   ", th.muted)];
    row.extend(spans);
    if crate::davinci::ui::run_width(&row) > width {
        let label = if state.on_review() {
            "Review".to_string()
        } else {
            format!("Question {} of {}", state.page + 1, state.questions.len())
        };
        return Line::from(truncate_run(
            vec![span("   ", th.muted), span(label, th.primary)],
            width,
        ));
    }
    Line::from(row)
}

fn question_rows(
    state: &DecisionModalState,
    question: &DecisionModalQuestion,
    inner: u16,
    th: &Theme,
) -> Vec<Line<'static>> {
    let mut body = Vec::new();
    if state.questions.len() > 1 {
        body.extend(section_detail(
            inner,
            th,
            &format!(
                "Question {} of {} · {}",
                state.page + 1,
                state.questions.len(),
                question.title
            ),
        ));
    }
    body.extend(section_detail(inner, th, &question.question));
    body.extend(section_detail(
        inner,
        th,
        &format!("Materiality: {}", question.materiality),
    ));
    if let Some(rule) = question.selection_rule() {
        body.extend(section_detail(inner, th, &rule));
    }

    if state.inspecting_evidence {
        body.extend(section_detail(inner, th, "--- Supporting Evidence ---"));
        if question.evidence_refs.is_empty() {
            body.extend(section_detail(inner, th, "No evidence refs attached"));
        } else {
            for ev in &question.evidence_refs {
                body.extend(section_detail(inner, th, &format!("• {ev}")));
            }
        }
        body.extend(section_detail(inner, th, "---------------------------"));
    }

    if !question.custom_only {
        for (idx, opt) in question.options.iter().enumerate() {
            let focused = idx == question.cursor;
            let mark = if question.multi_select {
                if question.checked[idx] {
                    "[x]"
                } else {
                    "[ ]"
                }
            } else if question.chosen == Some(idx) {
                "(•)"
            } else {
                "( )"
            };
            let mut label = format!("{mark} {}. {}", idx + 1, opt.label);
            if opt.recommended {
                label.push_str(" [Recommended]");
            }
            body.push(section_row(inner, th, focused, &label, ""));
            if focused {
                body.extend(section_detail(inner, th, &opt.explanation));
            }
        }
    }

    if question.allow_custom {
        let focused = question.on_custom();
        let filled = question.custom().is_some();
        let mark = if question.custom_only {
            ""
        } else if question.multi_select {
            if filled {
                "[x] "
            } else {
                "[ ] "
            }
        } else if question.chosen == Some(question.options.len()) {
            "(•) "
        } else {
            "( ) "
        };
        let prefix = format!("{mark}Other: ");
        let text = if focused {
            // Keep the cursor on screen however long the answer grows.
            let room = usize::from(inner).saturating_sub(3 + prefix.width()).max(4);
            cursor_window(&question.custom_text, question.custom_cursor, room)
        } else if question.custom_text.is_empty() {
            "(type your own answer)".to_string()
        } else {
            question.custom_text.clone()
        };
        body.push(section_row(
            inner,
            th,
            focused,
            &format!("{prefix}{text}"),
            "",
        ));
    }
    body
}

/// `text` with a `▏` at `cursor`, cut to `room` columns around the cursor:
/// the characters just before it always show, with `…` marking a cut.
fn cursor_window(text: &str, cursor: usize, room: usize) -> String {
    let cursor = cursor.min(text.len());
    let (before, after) = text.split_at(cursor);
    let full = format!("{before}▏{after}");
    if full.width() <= room {
        return full;
    }
    let mut head = String::new();
    let mut used = 2; // the cursor and a leading ellipsis
    for c in before.chars().rev() {
        let w = c.width().unwrap_or(0);
        if used + w > room {
            break;
        }
        used += w;
        head.insert(0, c);
    }
    let mut tail = String::new();
    for c in after.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > room {
            tail.push('…');
            break;
        }
        used += w;
        tail.push(c);
    }
    let lead = if head.len() < before.len() { "…" } else { "" };
    format!("{lead}{head}▏{tail}")
}

fn review_rows(state: &DecisionModalState, inner: u16, th: &Theme) -> Vec<Line<'static>> {
    let mut body = section_detail(inner, th, "Review your answers");
    for question in &state.questions {
        let (text, color) = match question.summary() {
            Some(summary) => (summary, th.text),
            None => ("not answered".to_string(), th.warning),
        };
        let row = vec![
            span("   ", th.muted),
            span(format!("{}: ", question.title), th.muted),
            span(text, color),
        ];
        body.push(Line::from(truncate_run(row, inner)));
    }
    body
}

pub fn lines(state: &DecisionModalState, width: u16, th: &Theme) -> Vec<Line<'static>> {
    let inset = 4u16.min(width.saturating_sub(40) / 2);
    let inner = width.saturating_sub(inset * 2).saturating_sub(4);
    let mut body = Vec::new();

    if state.questions.len() > 1 {
        body.push(progress_strip(state, inner, th));
    }
    let question = state.current();
    match question {
        Some(question) => body.extend(question_rows(state, question, inner, th)),
        None => body.extend(review_rows(state, inner, th)),
    }

    if let Some(error) = &state.error {
        body.push(Line::from(truncate_run(
            vec![span("   ! ", th.error), span(error.clone(), th.error)],
            inner,
        )));
    }

    let many = state.questions.len() > 1;
    let enter = if state.on_review() {
        "enter submit"
    } else if state.page < state.last_page() {
        "enter next"
    } else {
        "enter confirm"
    };
    let mut hints = vec![super::sheet::hint(th, enter)];
    if let Some(question) = question {
        if question.multi_select && !question.on_custom() {
            hints.push(super::sheet::hint(th, "space toggle"));
        }
        if !question.custom_only {
            hints.push(super::sheet::hint(th, "↑↓/1-N select"));
        }
    }
    if many {
        hints.push(super::sheet::hint(th, "tab/shift+tab question"));
    }
    let editing = state.editing_text();
    hints.push(super::sheet::hint(
        th,
        if editing { "ctrl+d defer" } else { "d defer" },
    ));
    if question.is_some() && !editing {
        hints.push(super::sheet::hint(
            th,
            if state.inspecting_evidence {
                "i hide evidence"
            } else {
                "i inspect evidence"
            },
        ));
    }
    body.push(hint_row(inner, &hints, Some("esc cancel"), th));

    let title = match (many, question) {
        (false, Some(question)) => format!("Question · {}", question.title),
        (true, Some(_)) => format!(
            "Questions · {} of {}",
            state.page + 1,
            state.questions.len()
        ),
        _ => "Questions · Review".into(),
    };
    Surface::section(width, th)
        .inset(inset)
        .title(vec![span(title, th.primary)])
        .rows(body.into_iter().map(|r| r.spans).collect())
        .lines()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::theme::{ColorDepth, Theme};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn press(state: &mut DecisionModalState, codes: &[KeyCode]) -> ModalFlow {
        let mut flow = ModalFlow::Open;
        for code in codes {
            flow = state.handle_key(key(*code));
        }
        flow
    }

    fn type_text(state: &mut DecisionModalState, text: &str) {
        for c in text.chars() {
            state.handle_key(key(KeyCode::Char(c)));
        }
    }

    fn option(id: &str, recommended: bool) -> DecisionModalOption {
        DecisionModalOption {
            id: id.into(),
            label: id.to_uppercase(),
            explanation: format!("About {id}"),
            recommended,
        }
    }

    fn database() -> DecisionModalQuestion {
        DecisionModalQuestion::new(
            "database",
            "Database",
            "Which database?",
            "Storage boundary",
            vec!["db.rs".into()],
            vec![option("postgres", true), option("sqlite", false)],
            true,
            false,
        )
    }

    fn features() -> DecisionModalQuestion {
        DecisionModalQuestion::new(
            "features",
            "Features",
            "Which features?",
            "Scope",
            vec!["lib.rs".into()],
            vec![
                option("auth", true),
                option("cache", false),
                option("logs", true),
                option("metrics", false),
            ],
            true,
            false,
        )
        .multi(1, 3)
    }

    fn single(question: DecisionModalQuestion) -> DecisionModalState {
        DecisionModalState::new("req-1", 1, vec![question])
    }

    fn text(lines: &[Line<'_>]) -> String {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn f02_enter_required() {
        assert!(!commits_decision("highlight", true));
        assert!(!commits_decision("enter", false));
        assert!(commits_decision("enter", true));
    }

    #[test]
    fn f02_arrow_and_digit_selection_never_answer() {
        let mut state = single(database());
        state.select_number(2);
        assert_eq!(state.questions[0].cursor, 1);
        assert!(!state.submitted);
        state.move_selection(-1);
        assert_eq!(state.questions[0].cursor, 0);
        assert!(!state.submitted);
        // Explicit Enter commits the focused option of a lone question.
        assert_eq!(
            state.confirm(),
            Some(DecisionResult::Answers(vec![(
                "database".into(),
                QuestionAnswer::Choice("postgres".into())
            )]))
        );
        assert!(state.submitted);
    }

    #[test]
    fn f02_enter_repeats_only_once() {
        let mut state = single(database());
        assert!(state.confirm().is_some());
        assert_eq!(state.confirm(), None);
        assert_eq!(state.commit(), None);
        assert!(!state.is_valid());
    }

    #[test]
    fn f02_esc_cancels_and_defer_is_distinct() {
        let mut cancel_state = single(database());
        assert_eq!(press(&mut cancel_state, &[KeyCode::Esc]), ModalFlow::Closed);
        assert_eq!(cancel_state.outcome, Some(DecisionResult::Cancel));
        assert_eq!(cancel_state.cancel(), None);

        let mut defer_state = single(database());
        assert_eq!(
            press(&mut defer_state, &[KeyCode::Char('d')]),
            ModalFlow::Closed
        );
        assert_eq!(defer_state.outcome, Some(DecisionResult::Defer));
        assert_eq!(defer_state.defer(), None);
    }

    #[test]
    fn f02_modal_traps_permission_cycle_key() {
        assert_ne!(decision_key(true, "shift_tab", false), "delegate");
        assert_eq!(decision_key(true, "escape", false), "cancel");
        assert_eq!(decision_key(true, "enter", false), "confirm");
        assert_eq!(decision_key(true, "d", false), "defer");
        assert_eq!(decision_key(true, "d", true), "edit");
        assert_eq!(decision_key(true, "i", false), "inspect");
        assert_eq!(decision_key(true, "space", false), "toggle");
        assert_eq!(decision_key(false, "shift_tab", false), "delegate");
        // Shift+Tab is consumed by the dialog rather than cycling modes.
        let mut state = single(database());
        let flow = state.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(flow, ModalFlow::Open);
        assert!(!state.submitted);
    }

    #[test]
    fn multi_select_toggles_with_space_and_keeps_option_order() {
        let mut state = single(features());
        // auth, then logs (row 3), then auth again to uncheck, then cache.
        press(
            &mut state,
            &[
                KeyCode::Char(' '),
                KeyCode::Down,
                KeyCode::Down,
                KeyCode::Char(' '),
                KeyCode::Up,
                KeyCode::Up,
                KeyCode::Char(' '),
                KeyCode::Down,
                KeyCode::Char(' '),
            ],
        );
        assert_eq!(state.questions[0].checked, [false, true, true, false]);
        assert!(!state.submitted, "space never submits");
        assert_eq!(press(&mut state, &[KeyCode::Enter]), ModalFlow::Closed);
        assert_eq!(
            state.outcome,
            Some(DecisionResult::Answers(vec![(
                "features".into(),
                QuestionAnswer::Choices {
                    choice_ids: vec!["cache".into(), "logs".into()],
                    custom_text: None
                }
            )]))
        );
    }

    #[test]
    fn digits_toggle_on_multi_select_and_pick_on_single_select() {
        let mut state = single(features());
        press(&mut state, &[KeyCode::Char('4'), KeyCode::Char('1')]);
        assert_eq!(state.questions[0].checked, [true, false, false, true]);
        press(&mut state, &[KeyCode::Char('4')]);
        assert_eq!(state.questions[0].checked, [true, false, false, false]);

        let mut state = single(database());
        press(&mut state, &[KeyCode::Char('2')]);
        assert_eq!(state.questions[0].chosen, Some(1));
        assert!(!state.submitted);
    }

    #[test]
    fn selection_bounds_block_submission_and_keep_input() {
        let mut state = single(features());
        // Nothing checked: Enter refuses and says why.
        assert_eq!(press(&mut state, &[KeyCode::Enter]), ModalFlow::Open);
        assert!(state.error.as_deref().unwrap().contains("at least 1"));
        // A fourth check is refused at max 3, and the first three stay.
        press(
            &mut state,
            &[
                KeyCode::Char('1'),
                KeyCode::Char('2'),
                KeyCode::Char('3'),
                KeyCode::Char('4'),
            ],
        );
        assert!(state.error.as_deref().unwrap().contains("at most 3"));
        assert_eq!(state.questions[0].checked, [true, true, true, false]);
        // Typed custom text counts as one selection too.
        press(&mut state, &[KeyCode::Char('3')]);
        state.move_selection(10);
        assert!(state.editing_text());
        type_text(&mut state, "audit trail");
        assert_eq!(state.questions[0].selection_count(), 3);
        assert_eq!(press(&mut state, &[KeyCode::Enter]), ModalFlow::Closed);
        assert_eq!(
            state.outcome,
            Some(DecisionResult::Answers(vec![(
                "features".into(),
                QuestionAnswer::Choices {
                    choice_ids: vec!["auth".into(), "cache".into()],
                    custom_text: Some("audit trail".into())
                }
            )]))
        );
    }

    #[test]
    fn several_questions_navigate_without_losing_answers_then_submit_together() {
        let mut state = DecisionModalState::new("req", 4, vec![database(), features()]);
        assert_eq!(state.page, 0);
        // Pick sqlite on question 1: Enter moves to question 2, not submit.
        assert_eq!(
            press(&mut state, &[KeyCode::Down, KeyCode::Enter]),
            ModalFlow::Open
        );
        assert_eq!(state.page, 1);
        press(&mut state, &[KeyCode::Char('1'), KeyCode::Char('3')]);
        // Back to question 1 and forward again: both answers are intact.
        press(&mut state, &[KeyCode::BackTab]);
        assert_eq!(state.page, 0);
        assert_eq!(state.questions[0].chosen, Some(1));
        press(&mut state, &[KeyCode::Tab]);
        assert_eq!(state.questions[1].checked, [true, false, true, false]);
        // Enter on the last question opens the review page, not a submit.
        assert_eq!(press(&mut state, &[KeyCode::Enter]), ModalFlow::Open);
        assert!(state.on_review());
        let review = text(&lines(
            &state,
            100,
            &Theme::da_vinci(ColorDepth::TrueColor, false),
        ));
        assert!(review.contains("Database: SQLITE"), "{review}");
        assert!(review.contains("Features: AUTH, LOGS"), "{review}");
        assert_eq!(press(&mut state, &[KeyCode::Enter]), ModalFlow::Closed);
        assert_eq!(
            state.outcome,
            Some(DecisionResult::Answers(vec![
                ("database".into(), QuestionAnswer::Choice("sqlite".into())),
                (
                    "features".into(),
                    QuestionAnswer::Choices {
                        choice_ids: vec!["auth".into(), "logs".into()],
                        custom_text: None
                    }
                ),
            ]))
        );
    }

    #[test]
    fn submitting_with_an_unanswered_question_jumps_to_it() {
        let mut state = DecisionModalState::new("req", 1, vec![database(), features()]);
        // Skip straight to review with Tab, answering nothing.
        press(&mut state, &[KeyCode::Tab, KeyCode::Tab]);
        assert!(state.on_review());
        assert_eq!(press(&mut state, &[KeyCode::Enter]), ModalFlow::Open);
        assert_eq!(state.page, 0);
        assert!(state.error.as_deref().unwrap().starts_with("Database:"));
        assert!(!state.submitted);
        // Navigation clears the message; answers typed so far stay.
        press(&mut state, &[KeyCode::Down]);
        assert_eq!(state.error, None);
    }

    #[test]
    fn defer_and_cancel_from_any_page_send_no_partial_answers() {
        for (code, expected) in [
            (KeyCode::Esc, DecisionResult::Cancel),
            (KeyCode::Char('d'), DecisionResult::Defer),
        ] {
            let mut state = DecisionModalState::new("req", 1, vec![database(), features()]);
            press(&mut state, &[KeyCode::Enter, KeyCode::Char('1')]);
            assert_eq!(press(&mut state, &[code]), ModalFlow::Closed);
            assert_eq!(state.outcome, Some(expected));
        }
    }

    #[test]
    fn typing_in_the_custom_field_owns_letters_space_and_arrows() {
        let mut state = single(database());
        press(&mut state, &[KeyCode::Down, KeyCode::Down]);
        assert!(state.editing_text());
        // `d`, `i`, digits and space are text here, not commands.
        type_text(&mut state, "id 2");
        assert!(!state.submitted);
        assert_eq!(state.questions[0].custom_text, "id 2");
        // Left/right edit inside the text, by whole characters.
        press(&mut state, &[KeyCode::Left, KeyCode::Left]);
        type_text(&mut state, "é");
        assert_eq!(state.questions[0].custom_text, "idé 2");
        press(&mut state, &[KeyCode::Right, KeyCode::Backspace]);
        assert_eq!(state.questions[0].custom_text, "idé2");
        // Ctrl+D still defers while typing.
        let flow = state.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert_eq!(flow, ModalFlow::Closed);
        assert_eq!(state.outcome, Some(DecisionResult::Defer));
    }

    #[test]
    fn paste_lands_in_the_custom_field_on_one_line_and_is_capped() {
        let mut state = single(database());
        state.paste("ignored outside the text field");
        assert_eq!(state.questions[0].custom_text, "");
        press(&mut state, &[KeyCode::Down, KeyCode::Down]);
        state.paste(
            "line one
line	two",
        );
        assert_eq!(state.questions[0].custom_text, "line one line two");
        state.paste(&"x".repeat(MAX_CUSTOM_BYTES));
        assert_eq!(state.questions[0].custom_text.len(), MAX_CUSTOM_BYTES);
        assert!(state.error.as_deref().unwrap().contains("limited"));
        // The draft survives; the next key clears the message.
        press(&mut state, &[KeyCode::Backspace]);
        assert_eq!(state.error, None);
        assert_eq!(state.questions[0].custom_text.len(), MAX_CUSTOM_BYTES - 1);
    }

    #[test]
    fn typing_a_custom_answer_chooses_it_whatever_key_leaves_the_question() {
        let mut state = DecisionModalState::new("req", 1, vec![database(), features()]);
        // Pick option 1, then type in Other and leave with Tab, not Enter.
        press(
            &mut state,
            &[KeyCode::Char(' '), KeyCode::Down, KeyCode::Down],
        );
        type_text(&mut state, "foo");
        press(&mut state, &[KeyCode::Tab]);
        assert_eq!(
            state.questions[0].answer(),
            Ok(QuestionAnswer::Custom("foo".into()))
        );
        // Clearing the text leaves the question unanswered, not stale.
        press(&mut state, &[KeyCode::BackTab]);
        for _ in 0..3 {
            press(&mut state, &[KeyCode::Backspace]);
        }
        assert!(state.questions[0].answer().is_err());
    }

    #[test]
    fn altgr_characters_reach_the_custom_answer() {
        let mut state = single(database());
        press(&mut state, &[KeyCode::Down, KeyCode::Down]);
        let altgr = KeyModifiers::CONTROL | KeyModifiers::ALT;
        for c in ['@', '{', '\\', 'd'] {
            state.handle_key(KeyEvent::new(KeyCode::Char(c), altgr));
        }
        assert_eq!(state.questions[0].custom_text, "@{\\d");
        assert!(!state.submitted);
    }

    #[test]
    fn a_long_custom_answer_keeps_the_cursor_visible() {
        let theme = Theme::da_vinci(ColorDepth::TrueColor, false);
        let mut state = single(database());
        press(&mut state, &[KeyCode::Down, KeyCode::Down]);
        type_text(&mut state, &"日本語のテキスト".repeat(10));
        type_text(&mut state, "END");
        for width in [30u16, 60, 100] {
            let rendered = text(&lines(&state, width, &theme));
            assert!(rendered.contains("END▏"), "width {width}: {rendered}");
        }
        // Cursor at the start: the head shows and the cut tail is marked.
        state.questions[0].custom_cursor = 0;
        let rendered = text(&lines(&state, 40, &theme));
        assert!(rendered.contains("Other: ▏日本"), "{rendered}");
        assert_eq!(cursor_window("abcdef", 3, 20), "abc▏def");
        assert_eq!(cursor_window("abcdefghij", 10, 6), "…ghij▏");
        assert_eq!(cursor_window("abcdefghij", 0, 6), "▏abc…");
    }

    #[test]
    fn an_empty_custom_answer_is_refused_without_losing_the_choice() {
        let mut state = single(database());
        press(&mut state, &[KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
        assert!(state.error.is_some());
        assert!(!state.submitted);
        type_text(&mut state, "  CockroachDB ");
        assert_eq!(press(&mut state, &[KeyCode::Enter]), ModalFlow::Closed);
        assert_eq!(
            state.outcome,
            Some(DecisionResult::Answers(vec![(
                "database".into(),
                QuestionAnswer::Custom("CockroachDB".into())
            )]))
        );
    }

    #[test]
    fn f02_resize_preserves_custom_draft() {
        let theme = Theme::da_vinci(ColorDepth::TrueColor, false);
        let custom_only = DecisionModalQuestion::new(
            "q-1",
            "Custom question",
            "Enter custom",
            "Materiality",
            vec!["spec.rs".into()],
            vec![],
            true,
            true,
        );
        let mut state = single(custom_only);
        type_text(&mut state, "Hi 日本");
        for width in [20, 40, 80, 120, 8] {
            assert!(!lines(&state, width, &theme).is_empty());
        }
        assert_eq!(state.questions[0].custom_text, "Hi 日本");
        assert_eq!(
            state.commit(),
            Some(DecisionResult::Answers(vec![(
                "q-1".into(),
                QuestionAnswer::Custom("Hi 日本".into())
            )]))
        );
    }

    #[test]
    fn rendering_fits_narrow_terminals_and_marks_state() {
        let theme = Theme::da_vinci(ColorDepth::TrueColor, false);
        let mut state = DecisionModalState::new("req", 1, vec![database(), features()]);
        press(&mut state, &[KeyCode::Enter, KeyCode::Char('1')]);
        let wide = text(&lines(&state, 100, &theme));
        assert!(wide.contains("✓ Database"), "{wide}");
        assert!(wide.contains("Question 2 of 2"), "{wide}");
        assert!(wide.contains("[x] 1. AUTH [Recommended]"), "{wide}");
        assert!(wide.contains("[ ] 2. CACHE"), "{wide}");
        assert!(wide.contains("Choose 1 to 3"), "{wide}");
        assert!(wide.contains("space toggle"), "{wide}");
        for width in [24u16, 40, 60] {
            for line in lines(&state, width, &theme) {
                let used = crate::davinci::ui::run_width(&line.spans);
                assert!(used <= width, "width {width}: {used} > {width}");
            }
        }
    }

    #[test]
    fn f02_recommendation_is_visibly_marked_but_never_auto_submitted() {
        let theme = Theme::da_vinci(ColorDepth::TrueColor, false);
        let state = single(database());
        let rendered = text(&lines(&state, 100, &theme));
        assert!(rendered.contains("[Recommended]"));
        assert!(rendered.contains("( ) 1. POSTGRES"));
        assert!(!state.submitted);
    }

    #[test]
    fn release_events_and_keys_after_submission_are_ignored() {
        let mut state = single(database());
        let release =
            KeyEvent::new_with_kind(KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Release);
        assert_eq!(state.handle_key(release), ModalFlow::Open);
        assert!(!state.submitted);
        press(&mut state, &[KeyCode::Enter]);
        let before = state.clone();
        press(
            &mut state,
            &[KeyCode::Down, KeyCode::Char('d'), KeyCode::Esc],
        );
        assert_eq!(state, before);
    }
}
