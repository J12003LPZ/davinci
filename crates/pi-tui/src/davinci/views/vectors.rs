//! `5b` — `memory-status`. The vector index itself, as opposed to `2b`, which
//! is one query against it.
//!
//! Records are grouped by the kind the extractor assigned, because that is
//! what decides both importance and what gets evicted first; each row is a
//! count with its share of the index, never a bare number (design.md §9). The
//! destructive action says what it would destroy and that it cannot be undone.
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/vectors.ex`.

use ratatui::text::{Line, Span};

use crate::davinci::model::Model;
use crate::davinci::ui::{blank, meter, span, span_strong, truncate_run, wrap, Surface, MEASURE};

const KIND: usize = 13;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width.min(MEASURE + 14);
    let Some(index) = model.vector_index.as_ref() else {
        return vec![Line::from(vec![span(
            "the vector index is not configured — vector memory is off",
            th.muted,
        )])];
    };

    let head = vec![
        Line::from(vec![
            span("this repository ", th.muted),
            span(index.repo.clone(), th.secondary),
            span(" holds ", th.muted),
            span(index.repo_records.clone(), th.primary),
            span(" of ", th.muted),
            span(index.total_records.clone(), th.text),
            span(" records", th.muted),
        ]),
        Line::from(vec![
            span("retrieval automatic", th.muted),
            span(" · ", th.border),
            span("at most ", th.muted),
            span(index.injection_cap.clone(), th.text),
            span(" injected per turn", th.muted),
            span(" · ", th.border),
            span("floor ", th.muted),
            span(index.floor.clone(), th.text),
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
            spans.extend(meter(*fraction, 22, th, Some(th.secondary)));
            spans.push(span(format!("  {note}"), th.border));
            Line::from(spans)
        })
        .collect();

    let where_it_lives = Surface::new(width, th)
        .title(vec![span("WHERE IT LIVES", th.secondary)])
        .rows(vec![
            vec![
                span("embeddings ", th.muted),
                span(index.embeddings.clone(), th.text),
                span(format!("  {}", index.embed_host), th.border),
            ],
            vec![
                span("vectors    ", th.muted),
                span(index.store.clone(), th.text),
                span(format!("  {}", index.collection), th.border),
            ],
            vec![
                span("extraction ", th.muted),
                span(index.extraction.clone(), th.text),
                span("  one call per turn, off the critical path", th.border),
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

    let mut footer: Vec<Line<'static>> = wrap(
        &format!(
            "memory-clear drops this repository's {} records. It asks first, and it cannot be undone.",
            index.repo_records
        ),
        MEASURE,
    )
    .into_iter()
    .map(|row| Line::from(vec![span(row, th.warning)]))
    .collect();
    footer.push(Line::from(vec![
        span("enter search", th.border),
        span(" · ", th.border),
        span("i reindex", th.border),
        span(" · ", th.border),
        span("t toggle automatic retrieval", th.border),
        span(" · ", th.border),
        span("esc close", th.border),
    ]));

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
        .map(|line| Line::from(truncate_run(line.spans, model.width)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::model::VectorIndex;
    use crate::davinci::theme::{ColorDepth, State, Theme};
    use crate::davinci::ui::run_width;

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        model.vector_index = Some(VectorIndex {
            repo: "davinci-rust".into(),
            repo_records: "6,914".into(),
            total_records: "18,402".into(),
            injection_cap: "1.5k tokens".into(),
            floor: "0.70".into(),
            kinds: vec![
                (
                    "decision".into(),
                    "1,482".into(),
                    0.48,
                    "importance 0.9".into(),
                ),
                (
                    "constraint".into(),
                    "311".into(),
                    0.10,
                    "never evicted".into(),
                ),
            ],
            embeddings: "ollama".into(),
            embed_host: "127.0.0.1:11434 · bge-small 384d".into(),
            store: "qdrant".into(),
            collection: "collection davinci-memoria · 3 shards".into(),
            extraction: "haiku".into(),
            config: "%USERPROFILE%\\.davinci\\vector-memory.json".into(),
            health: vec![
                (State::Done, "reindexed on the last commit · 4m ago".into()),
                (
                    State::Attention,
                    "7 records failed to embed · retried next reindex".into(),
                ),
            ],
        });
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn every_kind_row_carries_its_count_and_share() {
        let m = model(100);
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows
            .iter()
            .any(|row| row.contains("decision") && row.contains("1,482")));
        assert!(rows
            .iter()
            .any(|row| row.contains("constraint") && row.contains("never evicted")));
    }

    #[test]
    fn the_index_names_where_it_lives_and_its_health() {
        let m = model(100);
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows.iter().any(|row| row.contains("WHERE IT LIVES")));
        assert!(rows
            .iter()
            .any(|row| row.contains("ollama") && row.contains("127.0.0.1:11434")));
        assert!(rows.iter().any(|row| row.contains("HEALTH")));
        assert!(rows
            .iter()
            .any(|row| row.contains("7 records failed to embed")));
    }

    #[test]
    fn the_destructive_action_states_what_it_destroys() {
        let m = model(100);
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(rows
            .iter()
            .any(|row| row.contains("memory-clear drops this repository's 6,914")));
        assert!(rows.iter().any(|row| row.contains("cannot be undone")));
    }

    #[test]
    fn with_no_index_the_screen_says_so() {
        let mut m = model(100);
        m.vector_index = None;
        let rows: Vec<String> = lines(&m).iter().map(text).collect();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].contains("not configured"));
    }

    #[test]
    fn no_row_overflows_the_window_at_any_width() {
        for width in [72u16, 80, 100, 120, 160] {
            let m = model(width);
            for row in lines(&m) {
                assert!(
                    run_width(&row.spans) <= width,
                    "at {width}: {:?}",
                    text(&row)
                );
            }
        }
    }
}
