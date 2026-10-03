//! Vector-memory status: reported counts, configuration and health.
//! Absence of a snapshot does not imply that a service is disabled or healthy.

use super::chrome::thousands;
use super::sheet::{hint, status_meter, Composer, SheetChrome};
use crate::davinci::model::{Model, VectorIndex};
use crate::davinci::ui::{section_detail, section_heading, section_state, span};
use ratatui::text::Line;

fn retrieval_mode(index: &VectorIndex) -> &str {
    if index.retrieval_mode.is_empty() {
        "unknown"
    } else {
        &index.retrieval_mode
    }
}

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let Some(index) = &model.vector_index else {
        return section_detail(width, th, "Vector-memory status is unavailable. Check the memory configuration from the conversation.");
    };
    let mut rows = section_heading(width, th, &format!("Repository: {}", index.repo));
    for (label, value) in [
        ("Repository records", &index.repo_records),
        ("Total records", &index.total_records),
        ("Injection cap per turn", &index.injection_cap),
        ("Relevance floor", &index.floor),
    ] {
        if !value.is_empty() {
            rows.extend(section_detail(width, th, &format!("{label}: {value}")));
        }
    }
    rows.extend(section_detail(
        width,
        th,
        &format!("Retrieval: {}", retrieval_mode(index)),
    ));
    for (kind, count, fraction, note) in &index.kinds {
        let share = if fraction.is_finite() {
            format!(" · {:.0}%", fraction * 100.0)
        } else {
            String::new()
        };
        rows.extend(section_heading(
            width,
            th,
            &format!("{kind}: {count}{share}"),
        ));
        rows.extend(section_detail(width, th, note));
    }
    rows.extend(section_heading(width, th, "Storage and models"));
    for (label, value) in [
        ("Embeddings", &index.embeddings),
        ("Embedding host", &index.embed_host),
        ("Vector store", &index.store),
        ("Collection", &index.collection),
        ("Extraction", &index.extraction),
        ("Config", &index.config),
        ("Shards", &index.shards),
        ("Model dimensions", &index.model_dims),
    ] {
        if !value.is_empty() {
            rows.extend(section_detail(width, th, &format!("{label}: {value}")));
        }
    }
    rows.extend(section_heading(width, th, "Health"));
    if index.health.is_empty() {
        rows.extend(section_detail(width, th, "No health checks reported."));
    }
    for (state, detail) in &index.health {
        rows.extend(section_state(width, th, *state, detail));
    }
    rows.extend(section_detail(width, th, &index.retrieval_note));
    if !index.injected.is_empty() {
        rows.extend(section_detail(
            width,
            th,
            &format!("Injected this session: {}", index.injected),
        ));
    }
    rows
}

fn count(text: &str) -> Option<u64> {
    let digits: String = text.chars().filter(char::is_ascii_digit).collect();
    (!digits.is_empty() && text.chars().all(|ch| ch.is_ascii_digit() || ch == ','))
        .then(|| digits.parse().ok())
        .flatten()
}
fn short(value: u64) -> String {
    if (1_000..100_000).contains(&value) {
        format!("{:.1}k", value as f64 / 1_000.0)
    } else {
        thousands(value)
    }
}

pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let index = model.vector_index.as_ref();
    let share = index.and_then(|index| {
        let repo = count(&index.repo_records)?;
        let total = count(&index.total_records)?;
        (total > 0).then_some((repo, total))
    });
    SheetChrome {
        header_right: index
            .filter(|index| !index.total_records.is_empty())
            .map(|index| vec![span(format!("{} records", index.total_records), th.muted)])
            .unwrap_or_default(),
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
        hints: vec![hint(th, "↑↓ scroll")],
        escape: Some("esc close"),
        composer: Composer::Hidden,
        ..SheetChrome::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::{
        fixtures,
        theme::{ColorDepth, Theme},
        ui,
    };
    fn model(width: u16) -> Model {
        let mut m = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            32,
            false,
        );
        m.vector_index = Some(fixtures::vector_index());
        m
    }
    fn text(m: &Model) -> String {
        lines(m)
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }
    #[test]
    fn counts_configuration_kind_notes_and_health_remain_available() {
        let m = model(120);
        let index = m.vector_index.as_ref().unwrap();
        let drawn = text(&m);
        for value in [
            &index.repo,
            &index.repo_records,
            &index.total_records,
            &index.embeddings,
            &index.embed_host,
            &index.collection,
            &index.store,
            &index.config,
            &index.floor,
        ] {
            assert!(drawn.contains(value));
        }
        for (kind, count, _, note) in &index.kinds {
            for value in [kind, count, note] {
                assert!(drawn.contains(value));
            }
        }
        for (_, note) in &index.health {
            assert!(drawn.contains(note));
        }
        assert!(!drawn.contains("one call per turn") && !drawn.contains("it asks first"));
    }
    #[test]
    fn index_meter_requires_real_counts_and_a_nonzero_denominator() {
        assert_eq!(count("6,914"), Some(6914));
        assert_eq!(count("6.9k"), None);
        assert_eq!(count("unknown"), None);
        assert_eq!(short(6914), "6.9k");
        let mut m = model(80);
        m.vector_index.as_mut().unwrap().total_records = "0".into();
        assert!(chrome(&m).status_right.is_none());
        m.vector_index.as_mut().unwrap().retrieval_mode.clear();
        assert!(text(&m).contains("Retrieval: unknown"));
    }
    #[test]
    fn unavailable_empty_and_narrow_indexes_are_honest_and_bounded() {
        let mut m = model(80);
        m.vector_index.as_mut().unwrap().health.clear();
        assert!(text(&m).contains("No health checks"));
        m.vector_index = None;
        assert!(text(&m).contains("unavailable") && !text(&m).contains("memory is off"));
        for width in [0, 1, 20, 32, 40, 80, 120] {
            for row in lines(&model(width)) {
                assert!(ui::run_width(&row.spans) <= width);
            }
        }
    }
}
