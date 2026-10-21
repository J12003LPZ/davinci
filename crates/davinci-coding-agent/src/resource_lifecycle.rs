//! A small owned maintenance loop. Dropping its owner wakes and stops it.
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

#[derive(Debug)]
pub struct IdleReaper {
    stop: Arc<(Mutex<bool>, Condvar)>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl IdleReaper {
    pub fn start(interval: Duration, mut maintain: impl FnMut() -> bool + Send + 'static) -> Self {
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let signal = stop.clone();
        let thread = std::thread::spawn(move || loop {
            let stopped = signal.0.lock().unwrap_or_else(|e| e.into_inner());
            let (stopped, _) = signal
                .1
                .wait_timeout_while(stopped, interval, |stop| !*stop)
                .unwrap_or_else(|e| e.into_inner());
            if *stopped {
                break;
            }
            drop(stopped);
            if !maintain() {
                break;
            }
        });
        Self {
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for IdleReaper {
    fn drop(&mut self) {
        *self.stop.0.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.stop.1.notify_all();
        if let Some(thread) = self.thread.take() {
            if thread.thread().id() != std::thread::current().id() {
                let _ = thread.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dropping_owner_wakes_a_long_idle_wait() {
        let owner = IdleReaper::start(Duration::from_secs(3600), || {
            panic!("must be stopped before maintenance")
        });
        let start = std::time::Instant::now();
        drop(owner);
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}
