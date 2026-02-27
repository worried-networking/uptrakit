//! Shared NATS JetStream primitives for cross-controller and cross-service messaging.
//!
//! This crate provides the envelope format, subject scheme, and connection wrapper
//! used by both the controller's `NatsTransport` and the external scheduler's
//! `NatsSchedulerNotifier`.
//!
//! ## Subject scheme
//!
//! | Routing | Subject |
//! |---------|---------|
//! | Broadcast (no filter) | `uptrakit.events.broadcast` |
//! | Service-targeted | `uptrakit.events.service.<uuid>` |
//! | Capability-targeted | `uptrakit.events.capability.<cap>` |
//! | Controller events | `uptrakit.events.controller` |
//!
//! ## Stream configuration
//!
//! - Name: `UPTRAKIT_EVENTS`
//! - Subjects: `uptrakit.events.>`
//! - Max age: 24 hours
//! - Storage: File

pub mod connection;
pub mod envelope;
pub mod error;
pub mod subjects;

pub use connection::NatsConnection;
pub use envelope::NatsEventEnvelope;
pub use error::NatsError;
