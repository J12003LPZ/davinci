use davinci_coding_agent::design::commands::*;
#[test]
fn shared_command_grammar() {
    assert!(matches!(
        parse_design_command("/design").unwrap(),
        DesignCommand::List {}
    ));
    assert!(matches!(
        parse_design_command("/design-sync src").unwrap(),
        DesignCommand::Sync { .. }
    ));
    match parse_design_command("/design new \"Café  onboarding\" --kind product --variants 2")
        .unwrap()
    {
        DesignCommand::Create { input } => {
            assert_eq!(input.brief, "Café  onboarding");
            assert_eq!(input.variants, 2);
        }
        _ => panic!("create expected"),
    }
    match parse_design_command("/design -- open a cafe landing page").unwrap() {
        DesignCommand::Create { input } => assert_eq!(input.brief, "open a cafe landing page"),
        _ => panic!("create expected"),
    }
    for invalid in [
        "/design open",
        "/design new x --bad y",
        "/design new x --variants 4",
        "/design new x --variants 1 --variants 2",
        "/design new 'unterminated",
    ] {
        assert!(parse_design_command(invalid).is_err(), "{invalid}");
    }
}
#[test]
fn typed_commands_reject_forged_owners() {
    assert!(serde_json::from_str::<DesignCommand>(
        r#"{"operation":"list","owner_session":"foreign"}"#
    )
    .is_err());
}

#[test]
fn cli_design_owns_its_flags_and_preserves_unicode_quotes() {
    let brief = "Café's \"morning\" menu";
    let args = [
        "--offline",
        "design",
        "new",
        brief,
        "--kind",
        "product",
        "--variants",
        "2",
    ]
    .map(str::to_owned);
    let parsed = davinci_coding_agent::args::parse_args(&args);
    assert!(parsed.offline);
    assert!(parsed.unknown_flags.is_empty());
    assert_eq!(parsed.messages.len(), 1);
    match parse_design_command(&parsed.messages[0]).unwrap() {
        DesignCommand::Create { input } => {
            assert_eq!(input.brief, brief);
            assert_eq!(
                input.kind,
                davinci_coding_agent::design::types::DesignKind::Product
            );
            assert_eq!(input.variants, 2);
        }
        _ => panic!("create expected"),
    }
}

const ID: &str = "0f3c2a1e-5b7d-4c8e-9a10-2b3c4d5e6f70";

fn usage_error(line: &str) -> String {
    match parse_design_command(line) {
        Err(error) => format!("{error:?}"),
        Ok(command) => panic!("{line} parsed as {command:?}"),
    }
}

#[test]
fn a_reserved_verb_with_a_bad_argument_is_a_usage_error_never_a_brief() {
    for line in [
        "/design open \"\"",
        "/design open ''",
        "/design status \"\"",
        "/design status ''",
        "/design status 0f3c2a1e",
        "/design revise 0f3c2a1e make the CTA blue",
        "/design revise not-an-id make the CTA blue",
        "/design export 0f3c2a1e --revision 1 --destination /tmp/x",
        "/design verify oops",
        "/design select",
        "/design resume nope",
        "/design fork oops --revision 1",
        "/design restore oops --revision 1",
        "/design apply oops --revision 1",
        "/design list of tasks app",
        "/design sync",
        "/design open a cafe landing page",
    ] {
        let error = usage_error(line);
        assert!(error.contains("usage:"), "{line}: {error}");
    }
}

#[test]
fn briefs_that_start_with_a_verb_use_new_or_the_double_dash() {
    for line in [
        "/design new \"open a cafe landing page\"",
        "/design -- open a cafe landing page",
        "/design open-source dashboard",
        "/design a status page for the API",
    ] {
        match parse_design_command(line).unwrap() {
            DesignCommand::Create { input } => assert!(!input.brief.is_empty(), "{line}"),
            other => panic!("{line}: {other:?}"),
        }
    }
}

#[test]
fn an_uppercase_uuid_is_the_same_id() {
    let upper = ID.to_ascii_uppercase();
    match parse_design_command(&format!("/design open {upper}")).unwrap() {
        DesignCommand::Open { artifact_id } => assert_eq!(artifact_id.to_string(), ID),
        other => panic!("{other:?}"),
    }
    match parse_design_command(&format!("/design revise {upper} make the CTA blue")).unwrap() {
        DesignCommand::Revise {
            artifact_id, brief, ..
        } => {
            assert_eq!(artifact_id.to_string(), ID);
            assert_eq!(brief, "make the CTA blue");
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        parse_design_command(&format!("/design status {upper}")).unwrap(),
        DesignCommand::Status { .. }
    ));
    // Prose that merely looks long is left alone.
    match parse_design_command(&format!("/design revise {ID} keep {upper} as the brand id"))
        .unwrap()
    {
        DesignCommand::Revise { brief, .. } => assert!(brief.contains(&upper)),
        other => panic!("{other:?}"),
    }
}
