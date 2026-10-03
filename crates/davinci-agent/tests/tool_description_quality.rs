use davinci_agent::tools::{tool_specs, validate_builtin_tool_descriptions};

#[test]
fn builtin_descriptions_meet_length_and_content_rules() {
    let result = validate_builtin_tool_descriptions(&tool_specs());
    assert!(result.is_ok(), "{result:#?}");
}
