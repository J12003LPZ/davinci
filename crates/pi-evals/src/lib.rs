use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvalCase {
    pub name: String,
    pub prompt: String,
    pub expected_contains: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvalResult {
    pub name: String,
    pub passed: bool,
    pub output: String,
    pub duration_ms: u64,
}

pub struct EvalRunner {
    cases: Vec<EvalCase>,
}

impl EvalRunner {
    pub fn new(cases: Vec<EvalCase>) -> Self {
        Self { cases }
    }

    pub fn run_mock(&self) -> Vec<EvalResult> {
        self.cases
            .iter()
            .map(|c| {
                let mock_output = format!("Completed prompt: {}", c.prompt);
                let passed = c
                    .expected_contains
                    .iter()
                    .all(|expected| mock_output.contains(expected));
                EvalResult {
                    name: c.name.clone(),
                    passed,
                    output: mock_output,
                    duration_ms: 10,
                }
            })
            .collect()
    }
}
