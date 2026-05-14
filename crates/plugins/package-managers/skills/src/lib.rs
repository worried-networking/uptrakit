pub(crate) mod config;
pub(crate) mod error;
pub(crate) mod lock;
pub(crate) mod plugin;

pub use config::SkillsConfig;
pub use plugin::{DESCRIPTOR, SkillsPlugin, validate_identifier};
