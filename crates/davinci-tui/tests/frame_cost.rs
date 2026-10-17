//! What one typed character costs the shell: the key, then a whole frame.
//! Run with `cargo test --release -p davinci-tui --test frame_cost -- --ignored --nocapture`.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use davinci_tui::davinci::{
    app,
    model::{Entry, Model},
    theme::{ColorDepth, State, Theme},
};
use std::time::{Duration, Instant};

fn long_session(turns: usize) -> Model {
    let mut m = Model::new(
        Theme::da_vinci(ColorDepth::TrueColor, false),
        160,
        50,
        false,
    );
    for turn in 0..turns {
        m.transcript.push(Entry::user(&format!(
            "turn {turn}: fix the parser in crates/davinci-tui/src/davinci/app.rs"
        )));
        m.transcript.push(Entry::Gap);
        m.transcript.push(Entry::agent("davinci"));
        m.transcript.push(
            Entry::tool(
                State::Done,
                "Read",
                "crates/davinci-tui/src/davinci/runtime.rs",
                Some("0.1s"),
            )
            .with_output(&"    let x = compose(model);\n".repeat(30)),
        );
        m.transcript.push(Entry::prose(&format!(
            "Here is **what changed** in `src/lib.rs`:\n\n- one\n- two\n\n```rust\nfn main() {{\n    println!(\"{turn}\");\n}}\n```\n\n{}",
            "Some longer paragraph of prose that wraps across the terminal width more than once. ".repeat(6)
        )));
        m.transcript.push(Entry::Gap);
    }
    m
}

fn percentile(samples: &mut [Duration], p: f64) -> Duration {
    samples.sort();
    samples[((samples.len() as f64 - 1.0) * p) as usize]
}

#[test]
#[ignore = "timing probe, not a gate"]
fn typing_cost_in_a_long_session() {
    for turns in [10, 200, 1000] {
        let mut m = long_session(turns);
        // Warm the settled-rows cache, as the first frame of a session does.
        let _ = app::compose_frame(&m, m.height);
        let mut samples = Vec::new();
        for i in 0..300 {
            let ch = (b'a' + (i % 26) as u8) as char;
            let started = Instant::now();
            app::handle_key(&mut m, KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
            let frame = app::compose_frame(&m, m.height);
            let rows: Vec<String> = frame.lines.iter().map(ToString::to_string).collect();
            std::hint::black_box(rows);
            samples.push(started.elapsed());
        }
        let p50 = percentile(&mut samples, 0.5);
        let p95 = percentile(&mut samples, 0.95);
        let max = percentile(&mut samples, 1.0);
        println!("turns={turns:5}  key+frame p50={p50:?} p95={p95:?} max={max:?}");
    }
}
