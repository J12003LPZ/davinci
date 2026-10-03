# Behavior regression fixtures

Each `*.json` file is a named regression suite. A case records the issue,
responsible owner, and a normal behavior scenario. Regression cases are
intentionally small: reproduce the observed failure first, then add the
smallest fixture and requirement that would catch it.

The loader validates unique case IDs, known owners, non-empty requirements,
and repository fixture paths. The production wiring regression also checks
that JSON-mode traces expose the selected prompt profile and activated
capability rather than only testing prompt-construction internals.
