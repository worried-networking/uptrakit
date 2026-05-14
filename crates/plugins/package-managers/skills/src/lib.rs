pub(crate) mod config;
pub(crate) mod detection;
pub(crate) mod discovery;
pub(crate) mod error;
pub(crate) mod lock;
pub(crate) mod plugin;
pub(crate) mod releases;

pub use config::SkillsConfig;
pub use plugin::{DESCRIPTOR, SkillsPlugin, validate_identifier};
