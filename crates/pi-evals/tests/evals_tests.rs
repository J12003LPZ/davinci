use pi_evals::{EvalCase, EvalRunner};

#[test]
fn test_evals_runner() {
    let runner = EvalRunner::new(vec![EvalCase {
        name: "test-eval".to_string(),
        prompt: "Write a fibonacci function in Rust".to_string(),
        expected_contains: vec!["fibonacci".to_string()],
    }]);

    let results = runner.run_mock();
    assert_eq!(results.len(), 1);
    assert!(results[0].passed);
}
