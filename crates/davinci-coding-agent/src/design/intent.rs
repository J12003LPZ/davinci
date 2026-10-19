//! What a `/design` brief asks for, decided without a model call.
//!
//! The brief is free text. Its kind is inferred by a keyword classifier so the
//! same words always give the same kind, and an explicit `--kind` always wins.

use super::types::DesignKind;
use serde::Serialize;

/// Concepts generated when the brief does not say how many.
pub const DEFAULT_VARIANTS: u32 = 3;

/// What the user typed, before inference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesignIntent {
    pub brief: String,
    pub explicit_kind: Option<DesignKind>,
    pub variants: u32,
}

/// Where a resolved kind came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KindSource {
    /// `--kind` on the legacy `new` form.
    Explicit,
    /// A keyword in the brief.
    Inferred,
    /// Nothing in the brief matched; the documented default applies.
    Default,
}

/// The intent after inference: what the controller creates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedDesignBrief {
    pub kind: DesignKind,
    pub variants: u32,
    pub brief: String,
    pub source: KindSource,
}

/// The kind a brief gets when no keyword matches. Landing is what `/design`
/// produced before inference existed, so unclassifiable briefs behave as they
/// always did.
pub const FALLBACK_KIND: DesignKind = DesignKind::Landing;

/// Phrases checked in order; the first group with a hit wins. Product comes
/// first because a "dashboard landing view" is still an app screen, and
/// Document before Landing so "one-page report" is not taken for a landing page.
const PRODUCT: &[&str] = &[
    "dashboard",
    "admin panel",
    "admin console",
    "control panel",
    "analytics",
    "mobile app",
    "ios app",
    "android app",
    "web app",
    "app screen",
    "settings page",
    "settings screen",
    "onboarding flow",
    "sign in",
    "sign-in",
    "login",
    "checkout",
    "form",
    "inbox",
    "kanban",
    "crm",
    "data table",
    "chat interface",
    "chat ui",
    "user interface",
    "saas app",
];
const DOCUMENT: &[&str] = &[
    "report",
    "resume",
    "r\u{e9}sum\u{e9}",
    "cv ",
    "one-pager",
    "one pager",
    "whitepaper",
    "white paper",
    "newsletter",
    "proposal",
    "invoice",
    "case study",
    "press release",
    "document",
    "documentation",
    "slide deck",
    "pitch deck",
    "brochure",
    "handout",
    "poster",
    "flyer",
];
const LANDING: &[&str] = &[
    "landing page",
    "landing",
    "homepage",
    "home page",
    "marketing site",
    "marketing page",
    "pricing page",
    "waitlist",
    "portfolio",
    "coming soon",
    "product page",
    "website",
    "web site",
];

fn has(text: &str, words: &[&str]) -> bool {
    words.iter().any(|word| {
        // A bare word must stand alone ("form" is not "format"); phrases and
        // padded entries match as written.
        if word.contains(' ') || word.contains('-') || word.ends_with(' ') {
            return text.contains(word);
        }
        text.split(|c: char| !c.is_alphanumeric())
            .any(|token| token == *word || token.strip_suffix('s') == Some(word))
    })
}

/// The kind a brief asks for and how that was decided.
pub fn infer_kind(brief: &str) -> (DesignKind, KindSource) {
    let text = format!("{} ", brief.to_lowercase());
    if has(&text, PRODUCT) {
        return (DesignKind::Product, KindSource::Inferred);
    }
    if has(&text, DOCUMENT) {
        return (DesignKind::Document, KindSource::Inferred);
    }
    if has(&text, LANDING) {
        return (DesignKind::Landing, KindSource::Inferred);
    }
    (FALLBACK_KIND, KindSource::Default)
}

impl DesignIntent {
    /// Apply `--kind` if given, otherwise infer. Never fails: every brief gets
    /// a kind.
    pub fn resolve(self) -> ResolvedDesignBrief {
        let (kind, source) = match self.explicit_kind {
            Some(kind) => (kind, KindSource::Explicit),
            None => infer_kind(&self.brief),
        };
        ResolvedDesignBrief {
            kind,
            variants: self.variants,
            brief: self.brief,
            source,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn briefs_map_to_kinds() {
        for (brief, kind) in [
            (
                "Create a modern dashboard for AI agents",
                DesignKind::Product,
            ),
            ("a mobile app for tracking runs", DesignKind::Product),
            ("signup form with validation", DesignKind::Product),
            ("quarterly report for the board", DesignKind::Document),
            ("my r\u{e9}sum\u{e9}", DesignKind::Document),
            ("a landing page for a caf\u{e9}", DesignKind::Landing),
            ("portfolio site for a photographer", DesignKind::Landing),
        ] {
            assert_eq!(infer_kind(brief).0, kind, "{brief}");
        }
    }

    #[test]
    fn a_bare_word_is_not_a_substring() {
        // "format" must not read as "form", "platform" must not read as "form".
        assert_eq!(
            infer_kind("a tool for format conversion").1,
            KindSource::Default
        );
        assert_eq!(infer_kind("our platform story").1, KindSource::Default);
    }

    #[test]
    fn an_unclassifiable_brief_gets_the_documented_default() {
        let (kind, source) = infer_kind("something fresh and friendly");
        assert_eq!((kind, source), (FALLBACK_KIND, KindSource::Default));
    }

    #[test]
    fn product_wins_over_landing_and_the_result_is_deterministic() {
        let brief = "a dashboard landing view";
        assert_eq!(infer_kind(brief), infer_kind(brief));
        assert_eq!(infer_kind(brief).0, DesignKind::Product);
    }

    #[test]
    fn an_explicit_kind_overrides_inference() {
        let resolved = DesignIntent {
            brief: "a dashboard".into(),
            explicit_kind: Some(DesignKind::Document),
            variants: 2,
        }
        .resolve();
        assert_eq!(resolved.kind, DesignKind::Document);
        assert_eq!(resolved.source, KindSource::Explicit);
        assert_eq!(resolved.variants, 2);
    }
}
