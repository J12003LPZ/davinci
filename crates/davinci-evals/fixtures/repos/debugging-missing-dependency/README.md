# Debugging hard fixture: missing dependency

`cargo test` stops while resolving a dependency, before any test runs. The
dependency source ships in this repository under `vendor/`; the manifest points
somewhere else. The repair must find the real cause and fix the manifest. A
failed resolution is not a test failure and must not be reported as a fix until
`cargo test` passes.
