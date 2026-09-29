//! Fail-closed structured-output validation. No TypeScript counterpart.
use davinci_coding_agent::output_schema::{check_reply, load_schema, repair_prompt};
use serde_json::json;

#[test]
fn unsupported_and_malformed_constraints_never_certify_an_answer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("schema.json");
    for schema in [
        json!({"type":"string", "pattern":"^[0-9]+$"}),
        json!({"properties":{"x":{"minimum":10}}}),
        json!({"$ref":"#/missing"}),
        json!({"anyOf":[]}),
        json!({"required":[12]}),
        json!({"properties":{"x":42}}),
        json!({"type":["string",42]}),
        json!({"type":"string", "examples":"not-an-array"}),
    ] {
        assert!(check_reply(&schema, "\"abc\"").is_err(), "{schema}");
        std::fs::write(&path, schema.to_string()).unwrap();
        assert!(load_schema(&path).is_err(), "{schema}");
    }
}

#[test]
fn oversized_schema_is_rejected_at_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("schema.json");
    std::fs::write(&path, json!({"description":"x".repeat(40000)}).to_string()).unwrap();
    assert!(load_schema(&path).is_err());
}

#[test]
fn oversized_reply_and_repair_diagnostics_are_bounded() {
    assert!(check_reply(
        &json!({"type":"string"}),
        &format!("\"{}\"", "x".repeat(300000))
    )
    .is_err());
    let errors = vec!["x".repeat(5000); 100];
    assert!(repair_prompt(&errors).len() < 25000);
}

#[test]
fn validation_budget_exhaustion_is_not_an_anyof_match() {
    let mut alternatives = vec![json!(false); 10];
    alternatives.push(json!(true));
    let schema = json!({"type":"array","items":{"anyOf":alternatives}});
    let reply = serde_json::to_string(&vec![0; 2000]).unwrap();
    assert!(check_reply(&schema, &reply).is_err());
}

#[test]
fn numeric_rounding_cannot_certify_fractional_values_as_integers() {
    let schema = json!({"type":"integer"});
    for value in ["9007199254740992.5", "1.00000000000000000001", "1e-400"] {
        assert!(check_reply(&schema, value).is_err(), "{value}");
    }
    for value in ["1.0", "1e1", "-0.0", "123", "9007199254740993"] {
        assert!(check_reply(&schema, value).is_ok(), "{value}");
    }
    assert!(check_reply(&json!({"type":"string"}), "\"9007199254740992.5\"").is_ok());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("schema.json");
    std::fs::write(&path, r#"{"const":1.00000000000000000001}"#).unwrap();
    assert!(load_schema(&path).is_err());
}
