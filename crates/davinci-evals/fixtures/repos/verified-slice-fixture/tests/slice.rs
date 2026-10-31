use verified_slice::first_or_zero;

#[test]
fn empty_slices_yield_zero() {
    assert_eq!(first_or_zero(&[]), 0);
}

#[test]
fn non_empty_slices_yield_the_first_value() {
    assert_eq!(first_or_zero(&[7, 9]), 7);
}