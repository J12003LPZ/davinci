//! Aggregate metrics for comparing frontend design fingerprints.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::fingerprint::DesignFingerprint;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VisualDiversityReport {
    pub item_count: usize,
    pub fingerprint_diversity: f64,
    pub font_concentration: f64,
    pub palette_concentration: f64,
    pub repeated_card_concentration: f64,
    pub visual_verification_rate: f64,
}

/// Return the mean pairwise fingerprint distance in the inclusive range 0..=1.
pub fn suite_diversity_score(items: &[DesignFingerprint]) -> f64 {
    if items.len() < 2 {
        return 0.0;
    }

    let mut total = 0.0;
    let mut pairs = 0_usize;
    for (index, left) in items.iter().enumerate() {
        for right in items.iter().skip(index + 1) {
            total += fingerprint_distance(left, right);
            pairs += 1;
        }
    }
    (total / pairs as f64).clamp(0.0, 1.0)
}

/// Produce the cross-brief visual evaluation metrics required by the eval plan.
/// Concentration uses Herfindahl-Hirschman concentration over each brief's
/// primary font and primary palette color; repeated-card concentration is the
/// mean repeated-card ratio already captured by each fingerprint.
pub fn score_visual_suite(
    items: &[DesignFingerprint],
    visual_verification: &[bool],
) -> VisualDiversityReport {
    VisualDiversityReport {
        item_count: items.len(),
        fingerprint_diversity: suite_diversity_score(items),
        font_concentration: primary_concentration(items, |item| item.font_families.first()),
        palette_concentration: primary_concentration(items, |item| item.dominant_colors.first()),
        repeated_card_concentration: mean_repeated_card_ratio(items),
        visual_verification_rate: visual_verification_rate(visual_verification),
    }
}

pub fn visual_verification_rate(verified: &[bool]) -> f64 {
    if verified.is_empty() {
        return 0.0;
    }
    verified.iter().filter(|value| **value).count() as f64 / verified.len() as f64
}

fn fingerprint_distance(left: &DesignFingerprint, right: &DesignFingerprint) -> f64 {
    let distances = [
        set_distance(&left.font_families, &right.font_families),
        set_distance(&left.dominant_colors, &right.dominant_colors),
        set_distance(&left.border_radius_buckets, &right.border_radius_buckets),
        f64::from(left.uses_gradient != right.uses_gradient),
        scalar_distance(left.repeated_card_ratio, right.repeated_card_ratio, 1.0),
        scalar_distance(
            f64::from(left.all_caps_label_count),
            f64::from(right.all_caps_label_count),
            f64::from(
                left.all_caps_label_count
                    .max(right.all_caps_label_count)
                    .max(1),
            ),
        ),
        scalar_distance(
            f64::from(left.numbered_section_count),
            f64::from(right.numbered_section_count),
            f64::from(
                left.numbered_section_count
                    .max(right.numbered_section_count)
                    .max(1),
            ),
        ),
    ];
    distances.iter().sum::<f64>() / distances.len() as f64
}

fn set_distance<T: Ord + Clone>(left: &[T], right: &[T]) -> f64 {
    let left = left
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let right = right
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let union = left.union(&right).count();
    if union == 0 {
        return 0.0;
    }
    1.0 - left.intersection(&right).count() as f64 / union as f64
}

fn scalar_distance(left: f64, right: f64, scale: f64) -> f64 {
    if scale <= 0.0 {
        0.0
    } else {
        ((left - right).abs() / scale).clamp(0.0, 1.0)
    }
}

fn primary_concentration<F>(items: &[DesignFingerprint], value: F) -> f64
where
    F: Fn(&DesignFingerprint) -> Option<&String>,
{
    if items.is_empty() {
        return 0.0;
    }
    let mut counts = BTreeMap::<&str, usize>::new();
    for item in items {
        let primary = value(item).map(String::as_str).unwrap_or("<none>");
        *counts.entry(primary).or_insert(0) += 1;
    }
    counts
        .values()
        .map(|count| {
            let share = *count as f64 / items.len() as f64;
            share * share
        })
        .sum()
}

fn mean_repeated_card_ratio(items: &[DesignFingerprint]) -> f64 {
    if items.is_empty() {
        return 0.0;
    }
    items
        .iter()
        .map(|item| item.repeated_card_ratio.clamp(0.0, 1.0))
        .sum::<f64>()
        / items.len() as f64
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    fn fingerprint(font: &str, color: &str, gradient: bool) -> DesignFingerprint {
        DesignFingerprint {
            font_families: vec![font.to_string()],
            dominant_colors: vec![color.to_string()],
            border_radius_buckets: vec![8],
            uses_gradient: gradient,
            repeated_card_ratio: 0.5,
            all_caps_label_count: 1,
            numbered_section_count: 1,
        }
    }

    #[test]
    fn similar_fingerprints_are_less_diverse_than_different_fingerprints() {
        let base = fingerprint("inter", "#111111", false);
        let similar = base.clone();
        let different = fingerprint("space grotesk", "#ff6b35", true);

        assert_eq!(suite_diversity_score(&[]), 0.0);
        assert_eq!(suite_diversity_score(&[base.clone()]), 0.0);
        assert_eq!(suite_diversity_score(&[base.clone(), similar]), 0.0);
        assert!(suite_diversity_score(&[base, different]) > 0.0);
    }

    #[test]
    fn reports_concentration_motifs_and_verification_rate() {
        let items = vec![
            fingerprint("inter", "#111111", false),
            fingerprint("inter", "#111111", false),
            fingerprint("space grotesk", "#ff6b35", true),
        ];

        let report = score_visual_suite(&items, &[true, false, true]);

        assert_eq!(report.item_count, 3);
        assert!(report.fingerprint_diversity > 0.0);
        assert!((report.font_concentration - 5.0 / 9.0).abs() < f64::EPSILON);
        assert!((report.palette_concentration - 5.0 / 9.0).abs() < f64::EPSILON);
        assert!((report.repeated_card_concentration - 0.5).abs() < f64::EPSILON);
        assert!((report.visual_verification_rate - (2.0 / 3.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn fixture_fingerprints_distinguish_similar_and_different_briefs() {
        let first = tempdir().expect("first fixture directory");
        let similar = tempdir().expect("similar fixture directory");
        let different = tempdir().expect("different fixture directory");
        let shared = r#"
            <style>
              body { font-family: Inter, sans-serif; color: #111111; }
              .card { border-radius: 8px; background: #111111; }
            </style>
            <h1>BUILD WITH CARE</h1>
            <article class="card">ONE</article>
            <article class="card">TWO</article>
        "#;
        fs::write(first.path().join("index.html"), shared).expect("first fixture source");
        fs::write(
            similar.path().join("index.html"),
            shared.replace("BUILD WITH CARE", "BUILD WITH INTENT"),
        )
        .expect("similar fixture source");
        fs::write(
            different.path().join("index.html"),
            r#"
                <style>
                  body { font-family: "Space Grotesk"; color: #ff6b35; }
                  .panel { border-radius: 32px; background: linear-gradient(90deg, #ff6b35, #101820); }
                </style>
                <h1>MAKE IT MATTER</h1>
                <h2>01. INTRODUCTION</h2>
                <section class="panel">FEATURED</section>
            "#,
        )
        .expect("different fixture source");

        let fingerprints = [first, similar, different]
            .iter()
            .map(|fixture| crate::visual::fingerprint_frontend_source(fixture.path()).unwrap())
            .collect::<Vec<_>>();

        assert!(suite_diversity_score(&fingerprints[..2]) < suite_diversity_score(&fingerprints));
    }
}
