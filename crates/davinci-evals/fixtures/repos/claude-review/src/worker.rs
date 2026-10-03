use std::sync::{Arc, Mutex};

pub fn increment(value: Arc<Mutex<u64>>) {
    *value.lock().unwrap() += 1;
}
