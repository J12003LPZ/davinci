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
        "/design list extra",
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
