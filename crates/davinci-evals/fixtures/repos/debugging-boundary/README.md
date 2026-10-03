# Debugging hard fixture: boundary bug

Authorization and tenant boundaries must be enforced at the service boundary,
not bypassed by a permissive caller or presentation-layer check.
