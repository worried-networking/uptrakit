//! Distributed tracing context propagated through wire protocol messages.
//!
//! Carries W3C-compatible trace and span identifiers for correlating messages
//! across service boundaries. When `tracing-opentelemetry` is wired in later,
//! [`current_trace_context`] will extract IDs from the current span context;
//! until then it generates random trace IDs.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::limits::{
    MAX_SPAN_ID_LEN, MAX_TRACE_ID_LEN, WireValidate, WireValidationError, check_opt_string_len,
    check_string_len,
};

/// Distributed tracing context for correlating messages across service boundaries.
///
/// ## Wire format
///
/// ```json
/// {"trace_id":"0123456789abcdef0123456789abcdef","span_id":"0123456789abcdef"}
/// ```
///
/// - `trace_id`: 32 lowercase hex characters (128-bit identifier)
/// - `span_id`: 16 lowercase hex characters (64-bit identifier), omitted when absent
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceContext {
    /// 128-bit trace identifier encoded as 32 lowercase hex characters.
    pub trace_id: String,
    /// 64-bit span identifier encoded as 16 lowercase hex characters.
    /// `None` when no parent span is active.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub span_id: Option<String>,
}

impl TraceContext {
    /// Generate a new trace context with a random trace ID and no span ID.
    ///
    /// The trace ID is a 128-bit random value formatted as 32 lowercase hex
    /// characters (UUID v4 without hyphens).
    pub fn generate() -> Self {
        Self {
            trace_id: uuid::Uuid::new_v4().simple().to_string(),
            span_id: None,
        }
    }
}

impl Default for TraceContext {
    fn default() -> Self {
        Self::generate()
    }
}

impl fmt::Display for TraceContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.span_id {
            Some(span) => write!(f, "{}:{}", self.trace_id, span),
            None => f.write_str(&self.trace_id),
        }
    }
}

impl WireValidate for TraceContext {
    fn wire_validate(&self) -> Result<(), WireValidationError> {
        check_string_len(&self.trace_id, MAX_TRACE_ID_LEN, "trace_context.trace_id")?;
        check_opt_string_len(&self.span_id, MAX_SPAN_ID_LEN, "trace_context.span_id")?;
        Ok(())
    }
}

/// Returns the trace context for the current execution context.
///
/// Currently generates a new random trace context. When `tracing-opentelemetry`
/// is wired in, this function will extract trace/span IDs from the current span
/// context, making all existing propagation plumbing light up automatically.
pub fn current_trace_context() -> TraceContext {
    TraceContext::generate()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_produces_valid_trace_id() {
        let ctx = TraceContext::generate();
        assert_eq!(ctx.trace_id.len(), 32, "trace_id must be 32 hex chars");
        assert!(
            ctx.trace_id.chars().all(|c| c.is_ascii_hexdigit()),
            "trace_id must contain only hex characters"
        );
        assert!(ctx.span_id.is_none(), "generated context has no span_id");
    }

    #[test]
    fn generate_produces_unique_ids() {
        let a = TraceContext::generate();
        let b = TraceContext::generate();
        assert_ne!(a.trace_id, b.trace_id, "two generated IDs must differ");
    }

    #[test]
    fn display_without_span_id() {
        let ctx = TraceContext {
            trace_id: "0123456789abcdef0123456789abcdef".to_string(),
            span_id: None,
        };
        assert_eq!(ctx.to_string(), "0123456789abcdef0123456789abcdef");
    }

    #[test]
    fn display_with_span_id() {
        let ctx = TraceContext {
            trace_id: "0123456789abcdef0123456789abcdef".to_string(),
            span_id: Some("fedcba9876543210".to_string()),
        };
        assert_eq!(
            ctx.to_string(),
            "0123456789abcdef0123456789abcdef:fedcba9876543210"
        );
    }

    #[test]
    fn serde_roundtrip_with_span_id() {
        let ctx = TraceContext {
            trace_id: "0123456789abcdef0123456789abcdef".to_string(),
            span_id: Some("fedcba9876543210".to_string()),
        };
        let json = serde_json::to_string(&ctx).unwrap();
        assert!(json.contains("span_id"));
        let deserialized: TraceContext = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, ctx);
    }

    #[test]
    fn serde_roundtrip_without_span_id() {
        let ctx = TraceContext {
            trace_id: "0123456789abcdef0123456789abcdef".to_string(),
            span_id: None,
        };
        let json = serde_json::to_string(&ctx).unwrap();
        assert!(!json.contains("span_id"), "None span_id must be omitted");
        let deserialized: TraceContext = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, ctx);
    }

    #[test]
    fn deserialize_missing_span_id() {
        let json = r#"{"trace_id":"0123456789abcdef0123456789abcdef"}"#;
        let ctx: TraceContext = serde_json::from_str(json).unwrap();
        assert_eq!(ctx.trace_id, "0123456789abcdef0123456789abcdef");
        assert!(ctx.span_id.is_none());
    }

    #[test]
    fn wire_validate_valid() {
        let ctx = TraceContext {
            trace_id: "0123456789abcdef0123456789abcdef".to_string(),
            span_id: Some("fedcba9876543210".to_string()),
        };
        assert!(ctx.wire_validate().is_ok());
    }

    #[test]
    fn wire_validate_trace_id_too_long() {
        let ctx = TraceContext {
            trace_id: "a".repeat(33),
            span_id: None,
        };
        let err = ctx.wire_validate().unwrap_err();
        assert_eq!(err.field, "trace_context.trace_id");
    }

    #[test]
    fn wire_validate_span_id_too_long() {
        let ctx = TraceContext {
            trace_id: "0123456789abcdef0123456789abcdef".to_string(),
            span_id: Some("a".repeat(17)),
        };
        let err = ctx.wire_validate().unwrap_err();
        assert_eq!(err.field, "trace_context.span_id");
    }

    #[test]
    fn current_trace_context_generates_valid() {
        let ctx = current_trace_context();
        assert!(ctx.wire_validate().is_ok());
    }
}
