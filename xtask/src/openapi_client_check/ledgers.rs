//! Reviewed ledgers of intentional spec↔client divergences.

/// `operationId` -> client method name, for legitimate name divergences.
pub const RENAME_MAP: &[(&str, &str)] = &[
    ("token", "oauth_token"),
    ("deactivate_service", "remove_service"),
    // … populated in Task 9 (~30 entries).
];

/// operationIds intentionally without a client method.
pub const SPEC_ONLY: &[&str] = &[
    "oidc_callback",
    // … populated in Task 9 (~20 entries).
];

/// Client methods intentionally without a spec operation.
pub const CLIENT_ONLY: &[&str] = &[
    "raw_request",
    "stream_update_output",
    "stream_events",
    "stream_batch_progress",
    "healthz",
    // … populated in Task 9. `list_all_*` are NOT listed (see `is_list_all_companion`).
];

/// Normalized path templates present in `paths.rs` but absent from the spec.
pub const PATHS_CLIENT_ONLY: &[&str] = &[
    "/healthz",
    // … populated in Task 9 (events stream, surfaces path fns).
];

/// True if `method` is a `list_all_<x>` whose `list_<x>` sibling exists.
#[must_use]
pub fn is_list_all_companion(method: &str, all_methods: &[String]) -> bool {
    let Some(rest) = method.strip_prefix("list_all_") else {
        return false;
    };
    let sibling = format!("list_{rest}");
    all_methods.contains(&sibling)
}

/// Fail if a `CLIENT_ONLY` name also appears in `RENAME_MAP` values or `SPEC_ONLY`.
///
/// # Errors
/// Returns an error string naming the first double-booked entry.
pub fn validate_no_double_booking() -> Result<(), String> {
    for name in CLIENT_ONLY {
        if RENAME_MAP.iter().any(|(_, method)| method == name) || SPEC_ONLY.contains(name) {
            return Err(format!(
                "ledger double-booking: '{name}' is in CLIENT_ONLY and another ledger"
            ));
        }
    }
    Ok(())
}
