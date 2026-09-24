//! Test-only credential hooks. Compiled out of release builds; see the
//! `test-fixtures` feature in Cargo.toml.

pub const fn enabled() -> bool {
    cfg!(any(test, feature = "test-fixtures"))
}
