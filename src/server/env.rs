//! Reading the environment.
//!
//! Every setting has a working default, so nothing here has to be set to start
//! the app — these are the overrides, and each one is named after the type it
//! comes back as.

/// An override, or the default when it isn't set.
pub fn string(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

/// An override that can also be switched *off*: setting it empty means "none",
/// which is not the same as leaving it unset and getting the default.
pub fn optional(key: &str, default: &str) -> Option<String> {
    Some(string(key, default)).filter(|value| !value.trim().is_empty())
}

/// A numeric override. Anything unparseable falls back to the default, since
/// refusing to start over a typo in a tuning knob is worse than ignoring it.
pub fn number<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}
