//! Tests for the `impl_report_conversion!` macro.
//!
//! Exercises all three macro arms:
//! 1. Simple variant mapping (`Source => Target::Variant`)
//! 2. Closure-based transform (`Source => Target, |e| ...`)
//! 3. Multi-conversion syntax (comma-separated simple variant mappings)

use rootcause::prelude::*;
use uptrakit_shared_macros::impl_report_conversion;

// ── Test error types ─────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
#[error("source error: {0}")]
struct SourceError(String);

#[derive(Debug, thiserror::Error)]
#[error("another source error: {0}")]
struct AnotherSourceError(String);

#[derive(Debug, thiserror::Error)]
#[error("third source error")]
struct ThirdSourceError;

#[derive(Debug, thiserror::Error)]
enum TargetError {
    #[error("from source: {0}")]
    Source(#[from] SourceError),

    #[error("from another: {0}")]
    Another(#[from] AnotherSourceError),

    #[error("from third: {0}")]
    Third(#[from] ThirdSourceError),

    #[error("custom: {0}")]
    Custom(String),
}

// ── Macro invocations ────────────────────────────────────────────────

// Arm 1: simple variant mapping
impl_report_conversion!(SourceError => TargetError::Source);

// Arm 2: closure-based transform
impl_report_conversion!(AnotherSourceError => TargetError, |e| TargetError::Custom(e.to_string()));

// Arm 3: multi-conversion syntax (with trailing comma)
impl_report_conversion! {
    ThirdSourceError => TargetError::Third,
}

// ── Tests ────────────────────────────────────────────────────────────

#[test]
fn simple_variant_mapping() {
    let source_report: Report<SourceError> = report!(SourceError("test".into()));
    let target_report: Report<TargetError> = source_report.context_to();

    assert!(
        matches!(target_report.current_context(), TargetError::Source(_)),
        "expected TargetError::Source, got: {:?}",
        target_report.current_context()
    );
}

#[test]
fn closure_based_transform() {
    let source_report: Report<AnotherSourceError> =
        report!(AnotherSourceError("custom msg".into()));
    let target_report: Report<TargetError> = source_report.context_to();

    match target_report.current_context() {
        TargetError::Custom(msg) => {
            assert!(
                msg.contains("custom msg"),
                "expected message containing 'custom msg', got: {msg}"
            );
        }
        other => panic!("expected TargetError::Custom, got: {other:?}"),
    }
}

#[test]
fn multi_conversion_syntax() {
    let source_report: Report<ThirdSourceError> = report!(ThirdSourceError);
    let target_report: Report<TargetError> = source_report.context_to();

    assert!(
        matches!(target_report.current_context(), TargetError::Third(_)),
        "expected TargetError::Third, got: {:?}",
        target_report.current_context()
    );
}

#[test]
fn result_ext_context_to() {
    // Verify `.context_to()` works on `Result<T, Report<SourceError>>` via `ResultExt`.
    let result: Result<(), Report<SourceError>> = Err(report!(SourceError("from result".into())));
    let converted: Result<(), Report<TargetError>> = result.map_err(|r| r.context_to());

    assert!(converted.is_err());
    assert!(matches!(
        converted.unwrap_err().current_context(),
        TargetError::Source(_)
    ));
}
