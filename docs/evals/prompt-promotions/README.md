# Prompt promotion evidence

`PromptPromotionEvidence` is the machine-checkable contract for moving a
candidate prompt toward Stable. It records the candidate and base hashes, the
Core-200 and cross-family run IDs, bounded quality metrics, and hashes for the
persisted artifact manifests.

Each `artifact_manifest_hashes` entry has the form
`<run-id>=<sha256-of-run-id/manifest.json>`. The run must be a complete,
content-addressed `ArtifactRoot`; its manifest prompt hashes must match the
candidate and stable hashes in the evidence. The CLI rejects missing,
tampered, mismatched, or insufficient evidence:

```text
davinci-evals behavior promote-check --evidence <evidence.json> \
  --artifacts-root target/behavior-evals
```

Optional `--candidate-prompt-hash` and `--stable-prompt-hash` arguments bind the
check to hashes supplied by the promotion job. The same values may be supplied
by the job environment when the CLI is wrapped by a release workflow.
