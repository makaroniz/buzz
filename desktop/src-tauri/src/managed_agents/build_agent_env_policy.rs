/// Non-secret settings that internal release builds may embed through
/// `BUZZ_BUILD_AGENT_ENV`.
pub(crate) const ALLOWED_BUILD_AGENT_ENV_KEYS: &[&str] = &[
    "BUZZ_AGENT_THINKING_EFFORT",
    "DATABRICKS_HOST",
    "DATABRICKS_MODEL",
];

/// Return whether `key` is approved non-secret build configuration.
pub(crate) fn is_allowed_build_agent_env_key(key: &str) -> bool {
    ALLOWED_BUILD_AGENT_ENV_KEYS.contains(&key)
}
