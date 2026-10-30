use verified_parser::parse_port;

#[test]
fn accepts_surrounding_whitespace() {
    assert_eq!(parse_port(" 8080 "), 8080);
}

#[test]
fn rejects_invalid_input_with_zero() {
    assert_eq!(parse_port("not-a-port"), 0);
}

#[test]
fn rejects_out_of_range_ports() {
    assert_eq!(parse_port("70000"), 0);
}