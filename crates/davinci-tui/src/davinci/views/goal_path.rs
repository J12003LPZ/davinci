//! The goal path: the model's task list (`todo` tool) pinned under the
//! working line, as Claude Code keeps its todos in view while it works.
//! Tasks are numbered; the one in progress spins and is drawn in full ink,
//! finished ones are checked and struck through, the rest wait in muted ink.
//! It stays while a task is open or a turn runs, and steps aside once every
//! task is done and the turn has ended (the transcript keeps the record).
use crate::davinci::model::{Model, Step};
use crate::davinci::theme::State;
use crate::davinci::ui::{self, span, span_strong, MEASURE};
use ratatui::style::Modifier;
use ratatui::text::Line;

/// Rows kept for tasks before the rest fold into `… +N more`.
const MAX_TASKS: usize = 8;

/// Whether the panel is up.
pub fn visible(model: &Model) -> bool {
    !model.goal_path.is_empty()
        && (model.running || model.goal_path.iter().any(|step| step.state != State::Done))
}

/// The fewest rows the panel needs: the header, one task, and the two fold
/// markers a long list may take.
const MIN_ROWS: usize = 4;

/// At most `budget` rows; nothing when even [`MIN_ROWS`] do not fit.
pub fn lines(model: &Model, budget: usize) -> Vec<Line<'static>> {
    if !visible(model) || budget < MIN_ROWS.min(model.goal_path.len() + 1) {
        return Vec::new();
    }
    let th = &model.theme;
    let steps = &model.goal_path;
    let done = steps
        .iter()
        .filter(|step| step.state == State::Done)
        .count();
    let width = model.width.min(MEASURE + 6);
    let mut rows = vec![Line::from(vec![
        span("  Goal path", th.text),
        span(format!(" · {done}/{} done", steps.len()), th.muted),
    ])];
    let room = (budget - 1).min(MAX_TASKS);
    let (start, end) = window(steps, room);
    if start > 0 {
        rows.push(Line::from(span(
            format!("    … {start} done above"),
            th.muted,
        )));
    }
    for (index, step) in steps.iter().enumerate().take(end).skip(start) {
        let glyph = match step.state {
            // A task left active by an interrupted turn does not spin.
            State::Active if model.running => th.spinner(model.tick, model.animate).to_string(),
            State::Active => "◉".to_string(),
            State::Done => "✓".to_string(),
            _ => "○".to_string(),
        };
        let number = format!("{}. ", index + 1);
        let (ink, strike) = match step.state {
            State::Done => (th.muted, true),
            State::Active => (th.text, false),
            _ => (th.muted, false),
        };
        let mut task = span(step.verb.clone(), ink);
        if strike {
            task.style = task.style.add_modifier(Modifier::CROSSED_OUT);
        }
        let mut spans = vec![
            span("  ", th.muted),
            span_strong(format!("{glyph} "), th.state_color(step.state), th),
            span(number, th.muted),
        ];
        if step.state == State::Active {
            task.style = task.style.add_modifier(Modifier::BOLD);
        }
        spans.push(task);
        if step.state == State::Active {
            if let Some(target) = &step.target {
                spans.push(span(format!(" · {target}"), th.muted));
            }
        }
        rows.push(Line::from(ui::truncate_run(spans, width)));
    }
    if end < steps.len() {
        rows.push(Line::from(span(
            format!("    … +{} more", steps.len() - end),
            th.muted,
        )));
    }
    rows
}

/// The slice of tasks shown in `room` rows: everything when it fits, else a
/// window that opens one finished task before the active one, keeping a row
/// for each fold marker it needs.
fn window(steps: &[Step], room: usize) -> (usize, usize) {
    if steps.len() <= room {
        return (0, steps.len());
    }
    let focus = steps
        .iter()
        .position(|step| step.state == State::Active)
        .or_else(|| steps.iter().position(|step| step.state != State::Done))
        .unwrap_or(steps.len() - 1);
    // Both markers may be needed; each takes a task row.
    let shown = room.saturating_sub(2).max(1);
    // One finished task of context before the active one, when there is room.
    let lead = usize::from(shown >= 3);
    let start = focus.saturating_sub(lead).min(steps.len() - shown);
    let start = if start == 1 { 0 } else { start };
    let end = (start + shown + usize::from(start == 0)).min(steps.len());
    (start, end)
}

/// Rows the panel takes at `budget`.
pub fn height(model: &Model, budget: usize) -> usize {
    lines(model, budget).len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::theme::{ColorDepth, Theme};

    fn model(states: &[State]) -> Model {
        let mut model = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 100, 44, true);
        model.goal_path = states
            .iter()
            .enumerate()
            .map(|(index, state)| Step::new(*state, &format!("task {}", index + 1), None))
            .collect();
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn tasks_are_numbered_with_progress_and_state() {
        let mut m = model(&[State::Done, State::Active, State::Queued]);
        m.tick = 0;
        let rows: Vec<String> = lines(&m, 12).iter().map(text).collect();
        assert_eq!(rows[0], "  Goal path · 1/3 done");
        assert_eq!(rows[1], "  ✓ 1. task 1");
        assert!(rows[2].ends_with("2. task 2"), "{}", rows[2]);
        assert!(!rows[2].contains('○') && !rows[2].contains('✓'));
        assert_eq!(rows[3], "  ○ 3. task 3");
    }

    #[test]
    fn done_tasks_are_struck_through_and_the_active_one_is_bold() {
        let m = model(&[State::Done, State::Active]);
        let rows = lines(&m, 12);
        let done = rows[1].spans.last().unwrap();
        assert!(done.style.add_modifier.contains(Modifier::CROSSED_OUT));
        let active = rows[2].spans.last().unwrap();
        assert!(active.style.add_modifier.contains(Modifier::BOLD));
        assert!(!active.style.add_modifier.contains(Modifier::CROSSED_OUT));
    }

    #[test]
    fn it_shows_while_work_is_open_and_steps_aside_when_finished() {
        let mut m = model(&[State::Done, State::Done]);
        m.running = false;
        assert!(lines(&m, 12).is_empty(), "all done and idle");
        m.running = true;
        assert_eq!(lines(&m, 12).len(), 3, "the last check lands while running");
        let mut m = model(&[State::Done, State::Queued]);
        m.running = false;
        assert_eq!(lines(&m, 12).len(), 3, "an open task keeps it up");
        m.goal_path.clear();
        assert!(lines(&m, 12).is_empty());
        let m = model(&[State::Queued]);
        assert!(lines(&m, 1).is_empty(), "no room, no panel");
    }

    #[test]
    fn a_long_path_folds_around_the_active_task() {
        let mut states = vec![State::Done; 6];
        states.push(State::Active);
        states.extend([State::Queued; 7]);
        let m = model(&states);
        let rows: Vec<String> = lines(&m, 20).iter().map(text).collect();
        assert!(rows.len() <= MAX_TASKS + 1, "{rows:?}");
        assert_eq!(rows[0], "  Goal path · 6/14 done");
        assert_eq!(rows[1], "    … 5 done above");
        assert_eq!(rows[2], "  ✓ 6. task 6");
        assert!(rows[3].ends_with("7. task 7"));
        assert!(rows.last().unwrap().starts_with("    … +"), "{rows:?}");
        // Every task row is accounted for: shown, above, or more.
        let more: usize = rows
            .last()
            .unwrap()
            .trim_start_matches("    … +")
            .trim_end_matches(" more")
            .parse()
            .unwrap();
        let shown = rows.len() - 3;
        assert_eq!(5 + shown + more, 14);
    }

    #[test]
    fn a_short_budget_still_shows_the_active_task() {
        let mut states = vec![State::Done; 3];
        states.push(State::Active);
        states.extend([State::Queued; 3]);
        let m = model(&states);
        let rows: Vec<String> = lines(&m, 4).iter().map(text).collect();
        assert!(
            rows.iter().any(|row| row.ends_with("4. task 4")),
            "{rows:?}"
        );
        assert!(rows.len() <= 4, "{rows:?}");
    }

    #[test]
    fn it_never_takes_more_rows_than_its_budget() {
        for len in 1..20 {
            for active in 0..len {
                let states: Vec<State> = (0..len)
                    .map(|index| match index.cmp(&active) {
                        std::cmp::Ordering::Less => State::Done,
                        std::cmp::Ordering::Equal => State::Active,
                        std::cmp::Ordering::Greater => State::Queued,
                    })
                    .collect();
                let m = model(&states);
                for budget in 0..14 {
                    let rows: Vec<String> = lines(&m, budget).iter().map(text).collect();
                    assert!(
                        rows.len() <= budget,
                        "len {len} active {active} budget {budget}: {rows:?}"
                    );
                    if !rows.is_empty() {
                        let wanted = format!("{}. task {}", active + 1, active + 1);
                        assert!(
                            rows.iter().any(|row| row.ends_with(&wanted)),
                            "len {len} active {active} budget {budget}: {rows:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_active_task_stays_in_view_when_earlier_ones_are_still_open() {
        let mut states = vec![State::Queued; 12];
        states.push(State::Active);
        let m = model(&states);
        let rows: Vec<String> = lines(&m, 9).iter().map(text).collect();
        assert!(
            rows.iter().any(|row| row.ends_with("13. task 13")),
            "{rows:?}"
        );
    }

    #[test]
    fn an_active_task_only_spins_while_a_turn_runs() {
        let mut m = model(&[State::Active, State::Queued]);
        m.running = false;
        m.tick = 2;
        assert!(text(&lines(&m, 12)[1]).starts_with("  ◉ 1."));
    }
}
