//! SMTP email notification plugin.
//!
//! Delivers notifications via SMTP using `mail-send`. Per-channel configuration
//! contains only the recipient addresses; SMTP server credentials and sender
//! identity are supplied at delivery time from the merged global SMTP settings.

pub mod config;
pub mod extensions;
pub mod plugin;

pub use config::EmailChannelConfig;
pub use plugin::{
    DESCRIPTOR, EmailPlugin, SmtpSettingsSnapshot, merge_smtp_into_config, smtp_from_settings_map,
};
