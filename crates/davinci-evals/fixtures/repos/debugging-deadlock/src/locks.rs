use std::sync::{Arc, Mutex};

pub fn transfer(first: Arc<Mutex<u32>>, second: Arc<Mutex<u32>>) {
    let _a = first.lock().unwrap();
    let _b = second.lock().unwrap();
}

pub fn reconcile(first: Arc<Mutex<u32>>, second: Arc<Mutex<u32>>) {
    let _b = second.lock().unwrap();
    let _a = first.lock().unwrap();
}

#[cfg(test)]
mod tests {
    #[test]
    fn both_operations_acquire_locks_in_the_same_order() {
        let source = include_str!("locks.rs");
        let transfer = source
            .split("pub fn transfer")
            .nth(1)
            .unwrap()
            .split("pub fn reconcile")
            .next()
            .unwrap();
        let reconcile = source.split("pub fn reconcile").nth(1).unwrap();
        let first_then_second =
            |body: &str| body.find("first.lock()").unwrap() < body.find("second.lock()").unwrap();
        assert_eq!(
            first_then_second(transfer),
            first_then_second(reconcile),
            "lock acquisition order must be consistent"
        );
    }
}
