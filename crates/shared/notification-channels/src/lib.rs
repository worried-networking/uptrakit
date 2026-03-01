//! Pluggable notification channel implementations for Uptrakit.
//!
//! This crate provides the [`NotificationChannel`] trait and concrete
//! implementations for delivering notifications via different transports
//! (webhook, Telegram, email). Channels are registered in a
//! [`ChannelRegistry`] and looked up by type name at dispatch time.

mod channel;
#[cfg(feature = "email")]
mod email;
mod error;
mod registry;
#[cfg(feature = "telegram")]
mod telegram;
#[cfg(feature = "webhook")]
mod webhook;

pub use channel::{DeliveryMessage, MessageAction, NotificationChannel};
pub use error::ChannelError;
pub use registry::ChannelRegistry;

#[cfg(feature = "email")]
pub use email::EmailChannel;
#[cfg(feature = "telegram")]
pub use telegram::TelegramChannel;
#[cfg(feature = "webhook")]
pub use webhook::WebhookChannel;
