//! `DAVINCI_STARTUP_TRACE`: where a slow start goes, without a profiler.

use std::io::Write;

/// Appends `<ms since run start>  <stage>` to the file named by
/// `DAVINCI_STARTUP_TRACE`, so a slow start can be split into stages. Off, it
/// costs one environment lookup per stage. The clock starts at the first mark.
pub fn mark(stage: &str) {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let start = *START.get_or_init(std::time::Instant::now);
    let Some(path) = std::env::var_os("DAVINCI_STARTUP_TRACE") else {
        return;
    };
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{:>7} ms  {stage}", start.elapsed().as_millis());
    }
}
