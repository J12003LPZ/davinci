pub fn bounded(value: i32) -> i32 {
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_bounded() {
        assert!(bounded(101) <= 100);
    }
}
