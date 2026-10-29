use verified_dedupe::unique;

#[test]
fn keeps_first_occurrence_order() {
    assert_eq!(unique(&[1, 2, 1, 3, 2]), vec![1, 2, 3]);
}

#[test]
fn preserves_a_non_sorted_order() {
    assert_eq!(unique(&[3, 1, 3, 2]), vec![3, 1, 2]);
}

#[test]
fn leaves_unique_input_unchanged() {
    assert_eq!(unique(&[4, 5]), vec![4, 5]);
}