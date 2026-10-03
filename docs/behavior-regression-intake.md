# Behavior regression intake

Use this workflow when a user report, benchmark run, or live evaluation
reveals a behavior failure:

1. Minimize the request and capture the exact failing trace.
2. Assign one owner: runtime, prompt, capability router, tool description,
   model family, or evaluator.
3. Add the smallest fixture repository and a named suite under
   `crates/davinci-evals/fixtures/behavior/regressions/`.
4. Encode the expected behavior as scenario requirements and independent
   verification commands.
5. Prove Stable reproduces the failure before changing production code.
6. Fix the smallest responsible subsystem and add its regression test.
7. Run the focused regression suite, then the Core-200 suite.
8. Run the required Preview paired evaluation.
9. Promote only with the content-addressed evidence and rollback bundle.

Run the offline loader with:

```text
cargo test -p davinci-evals regression_suite
```

Regression fixtures must never contain credentials, network dependencies, or
claims that are not independently verified by the evaluator.
