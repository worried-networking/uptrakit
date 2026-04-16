//! Telegram notification plugin.
//!
//! Sends messages to a configured Telegram chat via the Bot API. Renders
//! action buttons as inline keyboard buttons.

pub mod config;
pub mod plugin;
pub mod surfaces;

pub use config::TelegramChannelConfig;
pub use plugin::{DESCRIPTOR, TelegramPlugin};
