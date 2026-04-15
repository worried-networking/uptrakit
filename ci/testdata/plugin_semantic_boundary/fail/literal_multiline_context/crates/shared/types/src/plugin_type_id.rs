use std::borrow::Cow;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PluginTypeId(Cow<'static, str>);

impl PluginTypeId {
    pub const fn from_static(s: &'static str) -> Self {
        Self(Cow::Borrowed(s))
    }
}

pub mod plugin_ids {
    use super::PluginTypeId;

    pub const RELEASES_GITHUB: PluginTypeId = PluginTypeId::from_static("releases_github");
    pub const RELEASES_GITLAB: PluginTypeId = PluginTypeId::from_static("releases_gitlab");
    pub const GENERIC_SHELL: PluginTypeId = PluginTypeId::from_static("generic_shell");
    pub const WEBHOOK: PluginTypeId = PluginTypeId::from_static("webhook");
    pub const TELEGRAM: PluginTypeId = PluginTypeId::from_static("telegram");

    pub const ALL: &[PluginTypeId] = &[
        RELEASES_GITHUB,
        RELEASES_GITLAB,
        GENERIC_SHELL,
        WEBHOOK,
        TELEGRAM,
    ];
}
