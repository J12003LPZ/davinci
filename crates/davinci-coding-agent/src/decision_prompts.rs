//! Structured questions for hosts that can only show one `select` or one
//! `input` at a time: RPC clients without the `decision` op and the hosted
//! legacy TUI. Both drive this one state machine, so a batch of questions and
//! multi-select answers behave the same everywhere.
//!
//! Questions are asked in order. Nothing is sent until the last one is
//! answered; cancelling or deferring at any prompt ends the whole dialog, so
//! a partial set of answers never reaches the agent.

use davinci_agent::decisions::{DecisionQuestion, HostDecisionAction};
use davinci_agent::{DecisionHostReply, DecisionHostRequest, DecisionHostResponse};
use std::collections::BTreeMap;

pub const CUSTOM: &str = "Custom response";
pub const DONE: &str = "Done";
pub const DEFER: &str = "Defer decision";
pub const CANCEL: &str = "Cancel";

/// A client that keeps answering with something unexpected cannot loop us.
const MAX_PROMPTS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionPrompt {
    Select { title: String, options: Vec<String> },
    Input { title: String, placeholder: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionStep {
    Prompt(DecisionPrompt),
    Done(DecisionHostResponse),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Choice {
    Option(String),
    Custom,
    Done,
    Defer,
    Cancel,
}

pub struct SequentialDecision {
    questions: Vec<DecisionQuestion>,
    index: usize,
    answers: BTreeMap<String, HostDecisionAction>,
    /// The current multi-select question's picks so far.
    picked: Vec<String>,
    custom: Option<String>,
    typing: bool,
    prompts: usize,
    event_prefix: &'static str,
}

impl SequentialDecision {
    pub fn new(request: &DecisionHostRequest, event_prefix: &'static str) -> Self {
        Self {
            questions: request.questions.clone(),
            index: 0,
            answers: BTreeMap::new(),
            picked: Vec::new(),
            custom: None,
            typing: false,
            prompts: 0,
            event_prefix,
        }
    }

    /// The question the current prompt belongs to.
    pub fn question(&self) -> &DecisionQuestion {
        &self.questions[self.index.min(self.questions.len().saturating_sub(1))]
    }

    pub fn prompt(&self) -> DecisionPrompt {
        let question = self.question();
        let progress = if self.questions.len() > 1 {
            format!("Question {} of {} · ", self.index + 1, self.questions.len())
        } else {
            String::new()
        };
        if self.typing {
            return DecisionPrompt::Input {
                title: format!("{progress}Enter custom response"),
                placeholder: "Custom answer...".into(),
            };
        }
        let mut title = format!("{progress}{}: {}", question.title, question.question);
        if question.multi_select {
            let (min, max) = question.selection_bounds();
            title.push_str(&format!(" (choose {min}-{max}, one at a time"));
            let mut so_far: Vec<String> = self
                .picked
                .iter()
                .map(|id| label_of(question, id))
                .collect();
            so_far.extend(self.custom.iter().map(|text| format!("\"{text}\"")));
            if !so_far.is_empty() {
                title.push_str(&format!("; picked: {}", so_far.join(", ")));
            }
            title.push(')');
        }
        DecisionPrompt::Select {
            title,
            options: self.choices().into_iter().map(|(label, _)| label).collect(),
        }
    }

    fn choices(&self) -> Vec<(String, Choice)> {
        let question = self.question();
        let mut choices: Vec<(String, Choice)> = question
            .options
            .iter()
            .filter(|option| !question.custom_only && !self.picked.contains(&option.id))
            .map(|option| {
                let label = if option.recommended {
                    format!("{} (recommended)", option.label)
                } else {
                    option.label.clone()
                };
                (label, Choice::Option(option.id.clone()))
            })
            .collect();
        let mut reserved = Vec::new();
        if question.allow_custom && self.custom.is_none() {
            reserved.push((CUSTOM.to_string(), Choice::Custom));
        }
        if question.multi_select && self.count() >= question.selection_bounds().0 {
            reserved.push((DONE.to_string(), Choice::Done));
        }
        reserved.push((DEFER.to_string(), Choice::Defer));
        reserved.push((CANCEL.to_string(), Choice::Cancel));
        // The host answers with a row's text, so every row's text must be
        // unique: a model-written label equal to "Cancel", "Done" or another
        // option's label gets its id appended rather than shadowing that row.
        let clashes: Vec<bool> = (0..choices.len())
            .map(|index| {
                let label = &choices[index].0;
                reserved.iter().any(|(r, _)| r == label)
                    || choices
                        .iter()
                        .enumerate()
                        .any(|(n, (other, _))| n != index && other == label)
            })
            .collect();
        for (choice, clash) in choices.iter_mut().zip(clashes) {
            if let (true, Choice::Option(id)) = (clash, &choice.1) {
                choice.0 = format!("{} [{id}]", choice.0);
            }
        }
        choices.extend(reserved);
        choices
    }

    fn count(&self) -> usize {
        self.picked.len() + usize::from(self.custom.is_some())
    }

    /// Feed the host's answer to the current prompt. `None` means the user
    /// dismissed it, which cancels the whole dialog.
    pub fn answer(&mut self, value: Option<&str>) -> DecisionStep {
        self.prompts += 1;
        if self.prompts > MAX_PROMPTS || self.questions.is_empty() {
            return DecisionStep::Done(DecisionHostResponse::Unavailable);
        }
        let Some(value) = value else {
            return DecisionStep::Done(DecisionHostResponse::Cancelled);
        };
        if self.typing {
            self.typing = false;
            let text = value.trim();
            if text.is_empty() {
                // Back to the list rather than an empty answer.
                return DecisionStep::Prompt(self.prompt());
            }
            if !self.question().multi_select {
                return self.record(HostDecisionAction::AnswerCustom(text.to_string()));
            }
            self.custom = Some(text.to_string());
            return self.after_multi_pick();
        }
        let question = self.question();
        let choice = self
            .choices()
            .into_iter()
            .find(|(label, _)| label == value)
            .map(|(_, choice)| choice)
            // Clients may answer with the option id, or a bare label when
            // exactly one open option carries it.
            .or_else(|| {
                let open = |o: &&davinci_agent::decisions::DecisionOptionInput| {
                    !self.picked.contains(&o.id)
                };
                let by_label: Vec<_> = question
                    .options
                    .iter()
                    .filter(open)
                    .filter(|o| o.label == value)
                    .collect();
                let unique_label = if by_label.len() == 1 {
                    by_label.first().copied()
                } else {
                    None
                };
                question
                    .options
                    .iter()
                    .filter(open)
                    .find(|o| o.id == value)
                    .or(unique_label)
                    .filter(|_| !question.custom_only)
                    .map(|o| Choice::Option(o.id.clone()))
            });
        match choice {
            None => DecisionStep::Done(DecisionHostResponse::Unavailable),
            Some(Choice::Cancel) => DecisionStep::Done(DecisionHostResponse::Cancelled),
            Some(Choice::Defer) => {
                DecisionStep::Done(DecisionHostResponse::Reply(DecisionHostReply::uniform(
                    &self.questions,
                    HostDecisionAction::Defer,
                    self.event_id(),
                    davinci_session::now_ms(),
                )))
            }
            Some(Choice::Custom) => {
                self.typing = true;
                DecisionStep::Prompt(self.prompt())
            }
            Some(Choice::Done) => self.finish_multi(),
            Some(Choice::Option(id)) => {
                if !self.question().multi_select {
                    return self.record(HostDecisionAction::AnswerChoice(id));
                }
                self.picked.push(id);
                self.after_multi_pick()
            }
        }
    }

    fn after_multi_pick(&mut self) -> DecisionStep {
        if self.count() >= self.question().selection_bounds().1 {
            return self.finish_multi();
        }
        DecisionStep::Prompt(self.prompt())
    }

    fn finish_multi(&mut self) -> DecisionStep {
        let question = self.question();
        let choice_ids = question
            .options
            .iter()
            .filter(|option| self.picked.contains(&option.id))
            .map(|option| option.id.clone())
            .collect();
        let custom_text = self.custom.take();
        self.picked.clear();
        self.record(HostDecisionAction::AnswerChoices {
            choice_ids,
            custom_text,
        })
    }

    fn record(&mut self, action: HostDecisionAction) -> DecisionStep {
        let id = self.question().id.clone();
        self.answers.insert(id, action);
        self.index += 1;
        self.picked.clear();
        self.custom = None;
        if self.index < self.questions.len() {
            return DecisionStep::Prompt(self.prompt());
        }
        DecisionStep::Done(DecisionHostResponse::Reply(DecisionHostReply {
            answers: std::mem::take(&mut self.answers),
            host_event_id: self.event_id(),
            answered_at_ms: davinci_session::now_ms(),
        }))
    }

    fn event_id(&self) -> String {
        format!("{}{}", self.event_prefix, davinci_session::now_ms())
    }
}

fn label_of(question: &DecisionQuestion, id: &str) -> String {
    question
        .options
        .iter()
        .find(|option| option.id == id)
        .map(|option| option.label.clone())
        .unwrap_or_else(|| id.to_string())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use davinci_agent::decisions::{DecisionKind, DecisionOptionInput, DecisionState};

    pub(crate) fn question(
        id: &str,
        options: &[&str],
        multi: Option<(usize, usize)>,
    ) -> DecisionQuestion {
        DecisionQuestion {
            id: id.into(),
            kind: DecisionKind::Scope,
            title: id.to_uppercase(),
            question: format!("Pick {id}"),
            materiality: "Material".into(),
            evidence_refs: vec![],
            evidence_fingerprints: BTreeMap::new(),
            options: options
                .iter()
                .enumerate()
                .map(|(n, option)| DecisionOptionInput {
                    id: option.to_string(),
                    label: option.to_uppercase(),
                    explanation: String::new(),
                    recommended: n == 0,
                })
                .collect(),
            allow_custom: true,
            custom_only: false,
            multi_select: multi.is_some(),
            min_selections: multi.map(|(min, _)| min),
            max_selections: multi.map(|(_, max)| max),
            plan_revision: 1,
            state: DecisionState::Open,
            answer: None,
        }
    }

    fn drive(flow: &mut SequentialDecision, answers: &[Option<&str>]) -> DecisionStep {
        let mut last = DecisionStep::Prompt(flow.prompt());
        for answer in answers {
            last = flow.answer(*answer);
        }
        last
    }

    fn reply(step: DecisionStep) -> BTreeMap<String, HostDecisionAction> {
        match step {
            DecisionStep::Done(DecisionHostResponse::Reply(reply)) => reply.answers,
            other => panic!("expected a reply, got {other:?}"),
        }
    }

    fn batch() -> DecisionHostRequest {
        DecisionHostRequest {
            questions: vec![
                question("db", &["postgres", "sqlite"], None),
                question("features", &["auth", "cache", "logs"], Some((1, 2))),
            ],
        }
    }

    #[test]
    fn a_batch_is_asked_in_order_and_answered_once_at_the_end() {
        let mut flow = SequentialDecision::new(&batch(), "t-");
        match flow.prompt() {
            DecisionPrompt::Select { title, options } => {
                assert!(title.starts_with("Question 1 of 2 · DB"), "{title}");
                assert_eq!(
                    options,
                    ["POSTGRES (recommended)", "SQLITE", CUSTOM, DEFER, CANCEL]
                );
            }
            other => panic!("{other:?}"),
        }
        let step = flow.answer(Some("SQLITE"));
        let DecisionStep::Prompt(DecisionPrompt::Select { title, options }) = step else {
            panic!("question two should follow");
        };
        assert!(title.contains("Question 2 of 2") && title.contains("choose 1-2"));
        // No Done until the minimum is met.
        assert!(!options.contains(&DONE.to_string()));
        let step = flow.answer(Some("logs"));
        let DecisionStep::Prompt(DecisionPrompt::Select { title, options }) = step else {
            panic!("one pick of two leaves the question open");
        };
        assert!(title.contains("picked: LOGS"), "{title}");
        assert!(!options.contains(&"LOGS".to_string()));
        assert!(options.contains(&DONE.to_string()));
        let answers = reply(flow.answer(Some("AUTH (recommended)")));
        assert_eq!(
            answers["db"],
            HostDecisionAction::AnswerChoice("sqlite".into())
        );
        // Max reached ends the question; ids come back in option order.
        assert_eq!(
            answers["features"],
            HostDecisionAction::AnswerChoices {
                choice_ids: vec!["auth".into(), "logs".into()],
                custom_text: None
            }
        );
    }

    #[test]
    fn custom_text_joins_a_multi_select_answer() {
        let mut flow = SequentialDecision::new(&batch(), "t-");
        let answers = reply(drive(
            &mut flow,
            &[
                Some("POSTGRES (recommended)"),
                Some(CUSTOM),
                Some("  audit  "),
                Some(DONE),
            ],
        ));
        assert_eq!(
            answers["features"],
            HostDecisionAction::AnswerChoices {
                choice_ids: vec![],
                custom_text: Some("audit".into())
            }
        );
    }

    #[test]
    fn defer_or_cancel_anywhere_never_sends_part_of_the_batch() {
        let mut flow = SequentialDecision::new(&batch(), "t-");
        let answers = reply(drive(&mut flow, &[Some("SQLITE"), Some(DEFER)]));
        assert_eq!(answers.len(), 2);
        assert!(answers.values().all(|a| *a == HostDecisionAction::Defer));

        for last in [Some(CANCEL), None] {
            let mut flow = SequentialDecision::new(&batch(), "t-");
            assert_eq!(
                drive(&mut flow, &[Some("SQLITE"), Some("auth"), last]),
                DecisionStep::Done(DecisionHostResponse::Cancelled)
            );
        }
    }

    #[test]
    fn option_labels_never_shadow_the_reserved_rows_or_each_other() {
        let mut q = question("q", &["a", "b", "c"], None);
        q.options[0].recommended = false;
        q.options[0].label = CANCEL.into();
        q.options[1].label = "Same".into();
        q.options[2].label = "Same".into();
        let request = DecisionHostRequest { questions: vec![q] };
        let flow = SequentialDecision::new(&request, "t-");
        let DecisionPrompt::Select { options, .. } = flow.prompt() else {
            panic!("select expected");
        };
        assert_eq!(
            options,
            ["Cancel [a]", "Same [b]", "Same [c]", CUSTOM, DEFER, CANCEL]
        );
        // The real Cancel row cancels; it is never recorded as option a.
        let mut cancel = SequentialDecision::new(&request, "t-");
        assert_eq!(
            cancel.answer(Some(CANCEL)),
            DecisionStep::Done(DecisionHostResponse::Cancelled)
        );
        // Each duplicate-label option is reachable; an ambiguous bare label
        // picks neither.
        let mut pick = SequentialDecision::new(&request, "t-");
        assert_eq!(
            reply(pick.answer(Some("Same [c]")))["q"],
            HostDecisionAction::AnswerChoice("c".into())
        );
        let mut ambiguous = SequentialDecision::new(&request, "t-");
        assert_eq!(
            ambiguous.answer(Some("Same")),
            DecisionStep::Done(DecisionHostResponse::Unavailable)
        );

        // A multi-select option labelled "Done" cannot fake finishing.
        let mut multi = question("m", &["x", "y"], Some((1, 2)));
        multi.options[1].label = DONE.into();
        let request = DecisionHostRequest {
            questions: vec![multi],
        };
        let mut flow = SequentialDecision::new(&request, "t-");
        flow.answer(Some("X (recommended)"));
        assert_eq!(
            reply(flow.answer(Some(DONE)))["m"],
            HostDecisionAction::AnswerChoices {
                choice_ids: vec!["x".into()],
                custom_text: None
            }
        );
    }

    #[test]
    fn unknown_answers_and_endless_clients_end_unavailable() {
        let mut flow = SequentialDecision::new(&batch(), "t-");
        assert_eq!(
            flow.answer(Some("MySQL")),
            DecisionStep::Done(DecisionHostResponse::Unavailable)
        );
        // An empty custom answer returns to the list, but not forever.
        let mut flow = SequentialDecision::new(&batch(), "t-");
        let mut step = DecisionStep::Prompt(flow.prompt());
        for _ in 0..200 {
            step = flow.answer(Some(CUSTOM));
            if let DecisionStep::Done(_) = step {
                break;
            }
            step = flow.answer(Some(""));
            if let DecisionStep::Done(_) = step {
                break;
            }
        }
        assert_eq!(step, DecisionStep::Done(DecisionHostResponse::Unavailable));
    }
}
