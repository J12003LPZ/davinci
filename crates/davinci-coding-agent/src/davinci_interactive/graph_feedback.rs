//! Announce terminal graph outcomes once, including while the sheet is closed.
use super::*;

pub fn refresh(model: &mut Model, sheet: GraphRunSheet) {
    let changed = !model
        .graph_run
        .as_ref()
        .is_some_and(|previous| previous.id == sheet.id && previous.phase == sheet.phase);
    if changed {
        if let Some(outcome) = sheet.outcome() {
            let mut message = format!("Graph {outcome}\nRun: {}\nGoal: {}", sheet.id, sheet.goal);
            if let Some(reason) = &sheet.blocked_reason {
                message.push_str(&format!("\nReason: {reason}"));
            }
            if !sheet.verification.is_empty() {
                message.push_str(&format!("\n{}", sheet.verification.join("\n")));
            }
            message.push_str(&format!(
                "\nElapsed: {} · Cost: {}",
                sheet.elapsed, sheet.cost
            ));
            if sheet.can_resume() {
                message.push_str(&format!(
                    "\nResume: /graph-resume {} (or press s in Graph Run).",
                    sheet.id
                ));
            }
            if !sheet.artifacts.is_empty() {
                message.push_str(&format!("\nDetails: {}", sheet.artifacts));
            }
            model.transcript.push(Entry::Gap);
            model
                .transcript
                .push(Entry::prose(&graph_public_text(&message)));
        }
    }
    davinci_tui::davinci::views::graph_nav::refresh(model, sheet);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_results_are_announced_once_without_changing_the_active_screen() {
        let mut model = Model::new(
            davinci_tui::davinci::theme::Theme::da_vinci(
                davinci_tui::davinci::theme::ColorDepth::TrueColor,
                false,
            ),
            120,
            30,
            false,
        );
        model.running = true; // A separate chat turn must not be stopped.
        let mut sheet = GraphRunSheet {
            id: "run-1".into(),
            phase: "implement".into(),
            ..Default::default()
        };
        let before = model.transcript.len();
        refresh(&mut model, sheet.clone());
        assert_eq!(model.transcript.len(), before);
        sheet.phase = "blocked".into();
        sheet.blocked_reason = Some("verification still failing after 3 revision cycles".into());
        refresh(&mut model, sheet.clone());
        assert_eq!(model.transcript.len(), before + 2);
        let rendered = format!("{:?}", model.transcript);
        assert!(rendered.contains("BLOCKED - goal not completed"));
        assert!(rendered.contains("verification still failing"));
        assert!(rendered.contains("/graph-resume run-1"));
        refresh(&mut model, sheet.clone());
        assert_eq!(model.transcript.len(), before + 2);
        assert_eq!(model.screen, Screen::Agent);
        assert!(model.running);
        // A resumed run can report its next outcome even with the same ID.
        sheet.phase = "implement".into();
        refresh(&mut model, sheet.clone());
        sheet.phase = "done".into();
        sheet.blocked_reason = None;
        refresh(&mut model, sheet.clone());
        assert_eq!(model.transcript.len(), before + 4);
        assert!(format!("{:?}", model.transcript).contains("Graph COMPLETED"));
        sheet.id = "run-2".into();
        sheet.phase = "cancelled".into();
        refresh(&mut model, sheet);
        assert_eq!(model.transcript.len(), before + 6);
        assert!(format!("{:?}", model.transcript).contains("Graph CANCELLED"));
    }
}
