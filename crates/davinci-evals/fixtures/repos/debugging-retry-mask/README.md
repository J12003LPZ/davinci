# Debugging hard fixture: deterministic failure masked by retry

The parser has a deterministic invalid-input failure. Retrying it or returning
a fallback hides the root cause and is not an accepted repair.
