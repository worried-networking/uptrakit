//! Dashboard Icons enhancement plugin.
//!
//! Automatically assigns icon URLs to software items by looking up their names
//! in the [Dashboard Icons](https://dashboardicons.com/) community project.
//!
//! The plugin listens for `SoftwareItemLifecycle` events and returns a
//! [`SoftwareItemPatch`] with the CDN URL when a matching icon slug exists.

pub mod cache;
pub mod config;
mod error;
pub mod plugin;
mod slugify;

pub use cache::DashboardIconCache;
pub use config::DashboardIconsConfig;
pub use plugin::{DESCRIPTOR, DashboardIconsPlugin};
