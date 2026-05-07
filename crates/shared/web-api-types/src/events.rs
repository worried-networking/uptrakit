//! SSE event types for real-time admin event streaming.
//!
//! [`AdminEvent`] is the server-side enum pushed over `GET /api/v1/events/stream`.
//! Each variant maps to an SSE `event:` name (via [`AdminEvent::event_name`]) with
//! the variant's inner fields serialised as the `data:` payload.
//!
//! The canonical definition lives in `uptrakit-wire::admin_events`. This module
//! re-exports it for backward compatibility with existing callers that import from
//! `uptrakit_web_api_types::events`.

pub use uptrakit_wire::admin_events::AdminEvent;
