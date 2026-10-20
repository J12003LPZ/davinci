//! An in-process Ollama for tests: answers `POST /api/embed` with vectors
//! chosen by keyword, so tests control which memories are near which queries.
//! Compiled only for tests and the `test-fixtures` feature.

use super::fake_qdrant::{read_request, respond};
use serde_json::{json, Value};
use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

pub struct FakeOllama {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    embeds: Arc<AtomicUsize>,
    thread: Option<JoinHandle<()>>,
}

impl FakeOllama {
    /// `rules` map a keyword to its vector; the first keyword found in a text
    /// (case-insensitive) wins. A text with no keyword gets the last axis.
    pub fn start(dimensions: usize, rules: Vec<(String, Vec<f32>)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake ollama");
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let embeds = Arc::new(AtomicUsize::new(0));
        let (worker_stop, worker_embeds) = (stop.clone(), embeds.clone());
        let thread = std::thread::spawn(move || {
            while !worker_stop.load(Ordering::SeqCst) {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(2));
                    continue;
                };
                let Some((method, path, _, body)) = read_request(&mut stream) else {
                    continue;
                };
                if method == "POST" && path == "/api/embed" {
                    worker_embeds.fetch_add(1, Ordering::SeqCst);
                    let embeddings: Vec<Value> = body["input"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default()
                        .iter()
                        .map(|text| {
                            let text = text.as_str().unwrap_or_default().to_lowercase();
                            let vector = rules
                                .iter()
                                .find(|(keyword, _)| text.contains(&keyword.to_lowercase()))
                                .map(|(_, vector)| vector.clone())
                                .unwrap_or_else(|| {
                                    let mut vector = vec![0.0; dimensions];
                                    if let Some(last) = vector.last_mut() {
                                        *last = 1.0;
                                    }
                                    vector
                                });
                            json!(vector)
                        })
                        .collect();
                    respond(&mut stream, 200, &json!({"embeddings": embeddings}));
                } else {
                    respond(&mut stream, 200, &json!({}));
                }
            }
        });
        Self {
            address,
            stop,
            embeds,
            thread: Some(thread),
        }
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.address)
    }

    /// How many `/api/embed` calls were answered.
    pub fn embed_calls(&self) -> usize {
        self.embeds.load(Ordering::SeqCst)
    }
}

impl Drop for FakeOllama {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
