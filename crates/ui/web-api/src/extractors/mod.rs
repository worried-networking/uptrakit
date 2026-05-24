pub mod etag_source;
pub mod if_match;

pub use etag_source::EtagSource;
pub use if_match::{GlobalSettingsVersion, IfMatch, SettingsVersion};
