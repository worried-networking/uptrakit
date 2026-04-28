//! Distributed tracing context propagated through wire protocol messages.
//!
//! Carries W3C-compatible trace and span identifiers for correlating messages
//! across service boundaries. When `tracing-opentelemetry` is wired in later,
//! [`current_trace_context`] will extract IDs from the current span context;
//! until then it generates random trace IDs.
use crate::generated::wire::limits::{
    MAX_SPAN_ID_LEN, MAX_TRACE_ID_LEN, WireValidate, WireValidationError, check_opt_string_len,
    check_string_len,
};
use serde::{Deserialize, Serialize};
use std::fmt;
/// Distributed tracing context for correlating messages across service boundaries.
///
/// ## Wire format
///
/// ```json
/// {"trace_id":"0123456789abcdef0123456789abcdef","span_id":"0123456789abcdef"}
/// ```ignore
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
