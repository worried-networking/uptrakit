/// Integration test for the `encrypted_column!` macro, exercised as a true
/// cross-crate invocation (this file is its own compilation unit, linking
/// `uptrakit_crypto` as an external crate) — the same situation every real
/// consumer (e.g. `uptrakit-shared-db`) is in. A same-crate instantiation
/// inside `uptrakit-crypto` itself was tried first and reproducibly hit a
/// rustc/clippy limitation: `#[expect(clippy::map_err_ignore, ...)]` inside
/// the macro's `ValueType::try_from` impl is not tracked as fulfilled when
/// the exported macro expands within its own defining crate, even though the
/// generated code is byte-for-byte identical to the real instantiations that
/// clippy-check clean. See the comment in `src/encrypted_column.rs`.
///
/// Runs in its own process (cargo compiles each `tests/*.rs` file into a
/// separate binary), so plaintext mode can be enabled unconditionally for the
/// whole binary without any save/restore dance.
use uptrakit_crypto::{CryptoError, encrypted_column};

encrypted_column!(
    /// Test-only encrypted column instantiation (plaintext-mode only).
    TestColumn,
    "uptrakit:test:column"
);

fn init() {
    uptrakit_crypto::enable_plaintext_mode();
}

#[test]
fn new_and_from_json_agree_on_as_json() {
    init();
    let via_new = TestColumn::new(r#"{"a":1}"#.to_string()).expect("valid json");
    let via_json = TestColumn::from_json(&serde_json::json!({"a": 1})).expect("from_json succeeds");
    assert_eq!(via_new.as_json(), via_json.as_json());
}

#[test]
fn partial_eq_value_cross_compares() {
    init();
    let col = TestColumn::new(r#"{"x":true}"#.to_string()).expect("valid json");
    assert_eq!(col, serde_json::json!({"x": true}));
}

#[test]
fn debug_output_is_redacted() {
    init();
    let col = TestColumn::new(r#"{"secret":"shh"}"#.to_string()).expect("valid json");
    assert_eq!(format!("{col:?}"), "TestColumn(***)");
}

#[test]
fn invalid_json_is_rejected() {
    let err = TestColumn::new("not valid json".to_string()).expect_err("must fail to parse");
    assert!(
        matches!(err.current_context(), CryptoError::InvalidJson(_)),
        "expected InvalidJson"
    );
}

/// Compile-time check that the generated type implements `TryGetable` even
/// wrapped in `Option<_>` — the shape SeaORM requires for LEFT-JOINed row
/// structs where the column may be `NULL`.
fn assert_try_getable<T: sea_orm::TryGetable>() {}

#[test]
fn try_getable_covers_optional_wrapper() {
    assert_try_getable::<Option<TestColumn>>();
}
