use verified_range::sum_range;

#[test]
fn includes_the_final_value() {
    assert_eq!(sum_range(1, 3), 6);
}

#[test]
fn a_single_value_range_sums_that_value() {
    assert_eq!(sum_range(4, 4), 4);
}