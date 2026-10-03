# Debugging hard fixture: swallowed errors

The service must preserve the upstream error when a dependency fails. A
passing response with an empty value is a symptom-suppression trap.
