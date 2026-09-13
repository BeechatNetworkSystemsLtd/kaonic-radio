//! Bring-up knobs read from the environment.
//!
//! Each of these overrides a compiled-in default so a board can be probed
//! without a rebuild; unset, the shipped value stands. A value that does not
//! parse falls back to the default rather than failing — a typo should not
//! stop a bring-up, and the setting it applies to is logged either way.

use std::str::FromStr;

/// `$name` parsed as `T`, or `default` when it is unset or unparseable.
pub fn var_or<T: FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Whether `$name` is set to `1`.
pub fn flag(name: &str) -> bool {
    std::env::var(name).as_deref() == Ok("1")
}
