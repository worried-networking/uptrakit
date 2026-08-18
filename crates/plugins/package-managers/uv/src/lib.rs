//! Uptrakit package-manager plugin for Python CLI tools installed via
//! [`uv tool install`](https://docs.astral.sh/uv/concepts/tools/).
//!
//! Scope: `uv tool` installs only. `uv pip` / system-environment packages are
//! out of scope — packages there are not attributable to uv (no install
//! receipt; indistinguishable from pip-installed).

pub mod config;
pub mod plugin;

pub use config::UvConfig;
pub use plugin::{parse_uv_tool_list, validate_identifier};
