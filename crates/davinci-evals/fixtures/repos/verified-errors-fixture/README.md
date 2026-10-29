# Error propagation fixture

`parse_positive` hides parse errors by returning `Ok(0)`. It must report an
error for non-numeric input and for zero.