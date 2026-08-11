use std::fmt;

/// Error returned when request field validation fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub field: &'static str,
    pub message: String,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

impl std::error::Error for ValidationError {}

/// Trait for validating request types before processing.
pub trait Validate {
    fn validate(&self) -> Result<(), ValidationError>;
}

pub(crate) mod sealed {
    pub trait Sealed {}
}
// pub(crate): the Sealed impl lives in the sibling surfaces.rs module (a
// private `mod sealed` would E0603 there); still unimplementable outside
// the crate, so the seal holds.

/// Routing envelope for both invoke paths: projected out of an
/// `Unvalidated<T>` body by [`RoutingEnvelope`], and built directly from the
/// GET query string by web-api's `split_get_envelope`
/// (`crates/ui/web-api/src/routes/surfaces.rs`) — one type, so the two paths
/// cannot drift apart.
/// Declared beside [`RoutingEnvelope`] so the carve-out's breadth is fixed
/// here: widening what pre-validation code can see means adding a field to
/// THIS struct — a reviewed change at the trait's own home, never a per-impl
/// decision.
#[derive(Debug, Clone)]
pub struct InvokeRoutingEnvelope {
    pub target_provider_id: Option<String>,
    pub timeout_seconds: Option<u16>,
}

/// Routing metadata a dispatcher may read from a body before validation.
/// Sealed: implementable only inside web-api-types, so declaring a type's
/// envelope is a reviewed change in the crate that owns the request types —
/// a doc-comment convention alone would be an ungated escape hatch from the
/// `Unvalidated<T>` type-state guarantee. Envelope fields are pure routing
/// inputs (they select a target; they are never business payload). No
/// associated type: an unconstrained `type Envelope` would let a future impl
/// return `Self` and hand the whole pre-validation body out — the concrete
/// return type bounds the projection at the trait, not per impl.
pub trait RoutingEnvelope: sealed::Sealed {
    fn routing_envelope(&self) -> InvokeRoutingEnvelope;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_error_display() {
        let err = ValidationError {
            field: "email",
            message: "must contain @".to_string(),
        };
        assert_eq!(err.to_string(), "email: must contain @");
    }
}
