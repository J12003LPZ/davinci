//! Context memory inspector view (`/context`).
//!
//! Provides a real-time inspection sheet for the prepared context manifest:
//! category, estimated tokens, mandatory/pinned/selected status, provenance,
//! freshness, source reference, and inclusion/exclusion reasons.

use super::sheet::{hint, Composer, SheetChrome};
use crate::davinci::model::Model;
use crate::davinci::ui::{section_detail, section_row, span};
use ratatui::text::Line;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let Some(sheet) = model
        .context_inspector
        .as_ref()
        .filter(|sheet| !sheet.rows.is_empty())
    else {
        return section_detail(
            width,
            th,
            "No context manifest available. Manifests appear after a provider request is prepared.",
        );
    };

    let mut rows = Vec::new();

    if let Some(dialog) = &sheet.confirmation_dialog {
        let mut dialog_lines =
            section_detail(width, th, &format!("[CONFIRMATION REQUIRED] {dialog}"));
        for line in &mut dialog_lines {
            for s in &mut line.spans {
                s.style.fg = Some(th.warning);
            }
        }
        rows.extend(dialog_lines);
    }

    for (i, item) in sheet.rows.iter().enumerate() {
        let is_selected = i == sheet.selected_index;

        let badge = if item.mandatory {
            "[mandatory]"
        } else if item.pinned {
            "[pinned]"
        } else if item.selected {
            "[selected]"
        } else {
            "[excluded]"
        };

        let title = format!("{badge} [{}] {}", item.category, item.item_id);
        let tokens_label = format!("~{} tok", item.estimated_tokens);

        rows.push(section_row(width, th, is_selected, &title, &tokens_label));

        let detail_str = format!(
            "  provenance: {} · freshness: {} · source: {}",
            item.provenance, item.freshness, item.source_ref
        );
        rows.extend(section_detail(width, th, &detail_str));

        if let Some(reason) = &item.inclusion_reason {
            let reason_str = format!("  reason: {reason}");
            rows.extend(section_detail(width, th, &reason_str));
        }

        if is_selected && sheet.preview_active {
            let preview = item
                .preview_body
                .as_deref()
                .unwrap_or("(no preview content available)");
            let mut preview_lines = section_detail(width, th, "  --- [PREVIEW] ---");
            // Paginate / bound preview to stay bounded
            for line in preview.lines().take(12) {
                preview_lines.extend(section_detail(width, th, &format!("    {line}")));
            }
            if preview.lines().count() > 12 {
                preview_lines.extend(section_detail(
                    width,
                    th,
                    "    [... remaining preview truncated to maintain bounds ...]",
                ));
            }
            for l in &mut preview_lines {
                for s in &mut l.spans {
                    s.style.fg = Some(th.secondary);
                }
            }
            rows.extend(preview_lines);
        }
    }

    rows
}

pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let (total_tokens, is_pending, source_rev, overlay_rev) = match &model.context_inspector {
        Some(sheet) => {
            let total = sheet
                .rows
                .iter()
                .filter(|r| r.selected)
                .map(|r| r.estimated_tokens)
                .sum::<u64>();
            (
                total,
                sheet.show_pending,
                sheet.source_revision,
                sheet.overlay_revision,
            )
        }
        None => (0, false, 0, 0),
    };

    let mode_str = if is_pending {
        format!("Pending next request (overlay rev {overlay_rev})")
    } else {
        format!("Current request (source rev {source_rev})")
    };

    SheetChrome {
        header_right: vec![span(
            format!("~{total_tokens} tokens (estimated prepared)"),
            th.muted,
        )],
        status_third: Some(vec![span(mode_str, th.muted)]),
        hints: vec![
            hint(th, "enter preview"),
            hint(th, "p pin"),
            hint(th, "x exclude"),
            hint(th, "r refresh"),
            hint(th, "tab toggle pending"),
            hint(th, "esc close"),
        ],
        escape: Some("esc close"),
        composer: Composer::Hidden,
        ..SheetChrome::default()
    }
}
