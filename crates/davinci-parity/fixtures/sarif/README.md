# Official SARIF schema fixture

`schema.json` is the unmodified OASIS SARIF 2.1.0 schema from
https://github.com/oasis-tcs/sarif-spec/blob/ed71d4f62db866ce3698a08a5ec3f7f2e775545d/sarif-2.1/schema/sarif-schema-2.1.0.json.
The upstream repository's license notice is retained in `LICENSE.md`.
Schema SHA-256: `c3b4bb2d6093897483348925aaa73af03b3e3f4bd4ca38cef26dcb4212a2682e`.

Used only by the explicit offline native exporter test. Install the pinned test
tools in an isolated Python environment using `scripts/security-sarif-requirements.txt`,
set `DAVINCI_SARIF_PYTHON` to that environment's Python executable, and run:

```
cargo test -p davinci-coding-agent --lib security_sarif_official_schema --locked -- --ignored
```

Validation never retrieves schemas over the network. Dependency installation is
a separate setup step. This validates interoperability structure and URI formats;
it does not prove vulnerability detection quality or every consumer's behavior.
