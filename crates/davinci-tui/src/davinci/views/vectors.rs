//! `5b` — `memory-status`. The vector index itself, as opposed to `2b`, which
//! is one query against it.
//!
//! Records are grouped by the kind the extractor assigned, because that is
//! what decides both importance and what gets evicted first; each row is a
//! count with its share of the index, never a bare number (design.md §9). The
//! destructive action says what it would destroy and that it cannot be undone.
//!
//! Mirrors artboard `5b` of `docs/ui/Pi TUI Instruments.dc.html`.

use ratatui::text::{Line, Span};

use super::chrome::thousands;
use super::sheet::{facts, hint, status_meter, Composer, SheetChrome};
use crate::davinci::model::Model;
use crate::davinci::ui::{blank, footnote, meter, span, span_strong, truncate_run, Surface};

const KIND: usize = 13;

fn retrieval_mode(index: &crate::davinci::model::VectorIndex) -> &str {
    if index.retrieval_mode.is_empty() {
        "unknown"
    } else {
        &index.retrieval_mode
    }
}

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let Some(index) = model.vector_index.as_ref() else {
        return vec![Line::from(vec![span(
            "the vector index is not configured — vector memory is off",
            th.muted,
        )])];
    };

    let head = vec![
        Line::from(vec![span_strong("VECTOR MEMORY", th.secondary, th)]),
        Line::from(vec![span(
            "Stored knowledge → relevant recall → turn context",
            th.muted,
        )]),
        blank(),
        Line::from(vec![
            span("this repository ", th.muted),
            span(index.repo.clone(), th.secondary),
            span(" holds ", th.muted),
            span(index.repo_records.clone(), th.primary),
            span(" of them", th.muted),
        ]),
        Line::from(vec![
            span(format!("retrieval {}", retrieval_mode(index)), th.muted),
            span(" · ", th.border),
            span("at most ", th.muted),
            span(index.injection_cap.clone(), th.text),
            span(" injected per turn", th.muted),
        ]),
    ];

    let rows: Vec<Line<'static>> = index
        .kinds
        .iter()
        .map(|(kind, count, fraction, note)| {
            let mut spans = vec![
                span(format!("{kind:<KIND$}"), th.muted),
                span(format!("{count:>6}  "), th.text),
            ];
            spans.extend(meter(
                *fraction,
                width.saturating_sub(24).min(22),
                th,
                Some(th.secondary),
            ));
            spans.push(span(format!("  {note}"), th.muted));
            Line::from(spans)
        })
        .collect();

    let where_it_lives = Surface::new(width, th)
        .title(vec![span("STORAGE & MODELS", th.secondary)])
        .rows(vec![
            vec![
                span("embeddings ", th.muted),
                span(index.embeddings.clone(), th.text),
                span(format!(" · {}", index.embed_host), th.border),
            ],
            vec![
                span("vectors    ", th.muted),
                span(index.store.clone(), th.text),
                span(format!(" · collection {}", index.collection), th.border),
            ],
            vec![
                span("extraction ", th.muted),
                span(index.extraction.clone(), th.text),
                span(" · one call per turn, off the critical path", th.border),
            ],
            vec![
                span("config     ", th.muted),
                span(index.config.clone(), th.border),
            ],
        ])
        .lines();

    let health = Surface::new(width, th)
        .title(vec![span("HEALTH", th.secondary)])
        .rows(
            index
                .health
                .iter()
                .map(|(state, text)| {
                    vec![
                        span_strong(format!("{} ", state.glyph()), th.state_color(*state), th),
                        span(text.clone(), th.muted),
                    ]
                })
                .collect::<Vec<Vec<Span<'static>>>>(),
        )
        .lines();

    let mut footer: Vec<Line<'static>> = Vec::new();
    let mut retrieval = vec![
        span("relevance floor ", th.muted),
        span(index.floor.clone(), th.text),
    ];
    if !index.retrieval_note.is_empty() {
        retrieval.push(span(" · ", th.border));
        retrieval.push(span(index.retrieval_note.clone(), th.border));
    }
    footer.push(Line::from(retrieval));
    if !index.injected.is_empty() {
        footer.push(Line::from(vec![
            span("injected this session ", th.muted),
            span(index.injected.clone(), th.text),
            span(" · shown in the transcript as ⌕ lines", th.border),
        ]));
    }
    footer.extend(footnote(
        width,
        vec![
            span("memory-clear", th.warning),
            span(
                format!(" drops this repo's {} records", index.repo_records),
                th.warning,
            ),
        ],
        vec![span("it asks first, and it cannot be undone", th.border)],
        th,
    ));

    let mut out = head;
    out.push(blank());
    out.extend(rows);
    out.push(blank());
    out.extend(where_it_lives);
    out.push(blank());
    out.extend(health);
    out.push(blank());
    out.extend(footer);
    out.into_iter()
        .map(|line| Line::from(truncate_run(line.spans, width)))
        .collect()
}

/// `6,914` → 6914. `None` for anything that is not a count.
fn count(text: &str) -> Option<u64> {
    let digits: String = text.chars().filter(char::is_ascii_digit).collect();
    (!digits.is_empty() && text.chars().all(|c| c.is_ascii_digit() || c == ','))
        .then(|| digits.parse().ok())
        .flatten()
}

/// The sheet's frame (design.md §11): the index in the header, retrieval
/// state in the status bar with this repository's share as its meter.
pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let index = model.vector_index.as_ref();
    let share = index.and_then(|index| {
        let repo = count(&index.repo_records)?;
        let total = count(&index.total_records)?;
        (total > 0).then_some((repo, total))
    });
    SheetChrome {
        header_right: facts(
            th,
            vec![
                index
                    .filter(|i| !i.total_records.is_empty())
                    .map(|i| vec![span(format!("{} records", i.total_records), th.muted)])
                    .unwrap_or_default(),
                index
                    .filter(|i| !i.shards.is_empty())
                    .map(|i| vec![span(format!("{} shards", i.shards), th.muted)])
                    .unwrap_or_default(),
                index
                    .filter(|i| !i.model_dims.is_empty())
                    .map(|i| vec![span(i.model_dims.clone(), th.muted)])
                    .unwrap_or_default(),
            ],
        ),
        status_third: index.map(|index| {
            vec![span(
                format!("retrieval {}", retrieval_mode(index)),
                th.muted,
            )]
        }),
        status_right: share.map(|(repo, total)| {
            status_meter(
                th,
                "index",
                repo as f64 / total as f64,
                &short(repo),
                &short(total),
            )
        }),
        hints: vec![
            hint(th, "/memory-search <query>"),
            hint(th, "/memory-reindex"),
            hint(th, "↑↓ scroll"),
        ],
        escape: Some("esc close"),
        composer: Composer::Prompt("/memory-search <query>"),
        echo: None,
    }
}

/// `6914` → `6.9k`, `18402` → `18.4k`: a decimal where it changes the reading.
fn short(value: u64) -> String {
    if (1_000..100_000).contains(&value) {
        format!("{:.1}k", value as f64 / 1_000.0)
    } else {
        thousands(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::fixtures;
    use crate::davinci::model::Screen;
    use crate::davinci::theme::{ColorDepth, Theme};
    use crate::davinci::ui::run_width;

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        model.vector_index = Some(fixtures::vector_index());
        model.toggle_screen(Screen::Vectors);
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn the_index_states_its_share_kinds_home_and_health() {
        let m = model(100);
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert_eq!(rows[0], "VECTOR MEMORY");
        assert_eq!(rows[3], "this repository davinci-rust holds 6,914 of them");
        assert!(rows[4].contains("at most 1.5k tokens injected per turn"));
        assert!(rows
            .iter()
            .any(|row| row.starts_with("decision") && row.contains("importance 0.9")));
        assert!(rows
            .iter()
            .any(|row| row.contains("embeddings ollama · 127.0.0.1:11434")));
        assert!(rows
            .iter()
            .any(|row| row.contains("vectors    qdrant · collection davinci-memoria")));
        assert!(rows.iter().any(|row| row.contains("HEALTH")));
        assert!(rows.iter().any(|row| row.contains("relevance floor 0.70")));
        assert!(rows.iter().any(|row| row
            .contains("memory-clear drops this repo's 6,914 records")
            && row.contains("it asks first, and it cannot be undone")));
        assert!(!rows.iter().any(|row| row.contains("esc close")));
    }

    #[test]
    fn no_index_says_memory_is_off() {
        let mut m = model(100);
        m.vector_index = None;
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows.iter().any(|row| row.contains("vector memory is off")));
    }

    #[test]
    fn the_sheet_wears_its_artboard_chrome() {
        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 100, 44, true);
        fixtures::dress_screen(&mut m, "5b");
        let c = chrome(&m);
        let header: String = c.header_right.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(header, "18,402 records │ 3 shards │ bge-small 384d");
        let third: String = c
            .status_third
            .as_deref()
            .unwrap()
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(third, "retrieval automatic");
        let right: String = c
            .status_right
            .as_deref()
            .unwrap()
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(right.starts_with("index "), "{right}");
        assert!(right.ends_with(" 6.9k/18.4k"), "{right}");
        assert_eq!(c.composer, Composer::Prompt("/memory-search <query>"));
        let hint = text(&super::super::sheet::hint_row(&m, &c).unwrap());
        assert!(
            hint.starts_with("/memory-search <query> │ /memory-reindex"),
            "{hint}"
        );
        assert!(hint.trim_end().ends_with("esc close"), "{hint}");
    }

    #[test]
    fn nothing_overflows_at_any_width() {
        for width in [72u16, 80, 100, 120, 160] {
            for row in lines(&model(width)) {
                assert!(
                    run_width(&row.spans) <= width,
                    "at {width}: {:?}",
                    text(&row)
                );
            }
        }
    }
}
