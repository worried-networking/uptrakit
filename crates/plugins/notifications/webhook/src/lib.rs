//! Webhook notification plugin.
//!
//! POSTs a JSON payload to the configured URL. Optionally signs the payload
//! with HMAC-SHA256 and includes the signature in the `X-Uptrakit-Signature`
//! header.

pub mod config;
pub mod plugin;
pub mod surfaces;

pub use config::WebhookChannelConfig;
pub use plugin::{DESCRIPTOR, WebhookPlugin};
