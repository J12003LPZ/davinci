use verified_errors::parse_positive;

#[test]
fn parses_a_positive_number() {
    assert_eq!(parse_positive("12"), Ok(12));
}

#[test]
fn rejects_non_numeric_input() {
    assert!(parse_positive("nope").is_err());
}

#[test]
fn rejects_zero() {
    assert!(parse_positive("0").is_err());
}