//! `/design` from a plain sentence: what the command keeps, what it infers,
//! and how a failed generation reports itself.

use davinci_coding_agent::design::commands::{parse_design_command, DesignCommand};
use davinci_coding_agent::design::types::DesignKind;

fn create(line: &str) -> davinci_coding_agent::design::records::CreateDesign {
    match parse_design_command(line).unwrap_or_else(|error| panic!("{line}: {error:?}")) {
        DesignCommand::Create { input } => input,
        other => panic!("{line}: create expected, got {other:?}"),
    }
}

#[test]
fn design_brief_without_flags_keeps_full_intent() {
    let input = create("/design Create a modern dashboard for AI agents");
    assert_eq!(input.brief, "Create a modern dashboard for AI agents");
    assert_eq!(
        input.kind,
        DesignKind::Product,
        "inferred, not implicit landing"
    );
    assert_eq!(input.variants, 3);

    // Legacy form: explicit flags override inference and parse as before.
    let legacy = create("/design new \"a dashboard\" --kind document --variants 2");
    assert_eq!(legacy.kind, DesignKind::Document);
    assert_eq!(legacy.variants, 2);
    assert_eq!(legacy.brief, "a dashboard");
}

#[test]
fn an_apostrophe_inside_prose_is_not_a_quote() {
    let input = create("/design Create a dashboard for Julien's team");
    assert_eq!(input.brief, "Create a dashboard for Julien's team");
    let inch = create("/design A 5\" screen kiosk app");
    assert_eq!(inch.brief, "A 5\" screen kiosk app");
    // A quote at the start of a word still quotes.
    let quoted = create("/design new 'Julien\\'s  team' --kind product");
    assert_eq!(quoted.brief, "Julien's  team");
    assert!(parse_design_command("/design new 'unterminated").is_err());
}

#[test]
fn a_brief_may_start_with_a_subcommand_word() {
    for (line, brief) in [
        (
            "/design open a cafe landing page",
            "open a cafe landing page",
        ),
        ("/design list of tasks app", "list of tasks app"),
        ("/design revise the hero section", "revise the hero section"),
        (
            "/design status board for my team",
            "status board for my team",
        ),
        ("/design export settings page", "export settings page"),
    ] {
        assert_eq!(create(line).brief, brief, "{line}");
    }
    // A subcommand with its real arguments is still a subcommand.
    let id = "2f6f2c2e-52c0-4d3a-8a4e-5b2f3c9a1d10";
    assert!(matches!(
        parse_design_command(&format!("/design open {id}")).unwrap(),
        DesignCommand::Open { .. }
    ));
    assert!(matches!(
        parse_design_command("/design list").unwrap(),
        DesignCommand::List {}
    ));
    // Bare verbs that need an argument are errors, not briefs.
    for bare in [
        "/design open",
        "/design status",
        "/design revise",
        "/design apply",
    ] {
        assert!(parse_design_command(bare).is_err(), "{bare}");
    }
}

#[test]
fn the_kind_comes_from_the_words_and_falls_back_to_the_documented_default() {
    assert_eq!(create("/design a signup form").kind, DesignKind::Product);
    assert_eq!(
        create("/design a quarterly report").kind,
        DesignKind::Document
    );
    assert_eq!(
        create("/design a landing page for a bakery").kind,
        DesignKind::Landing
    );
    assert_eq!(
        create("/design something fresh").kind,
        davinci_coding_agent::design::intent::FALLBACK_KIND
    );
    assert_eq!(create("/design a dashboard").variants, 3);
}
