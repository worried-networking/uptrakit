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

    pub const GENERIC_SHELL: PluginTypeId = PluginTypeId::from_static("generic_shell");
    pub const ENHANCEMENT_DASHBOARD_ICONS: PluginTypeId =
        PluginTypeId::from_static("enhancement_dashboard_icons");
    pub const ALL: &[PluginTypeId] = &[
        GENERIC_SHELL,
        ENHANCEMENT_DASHBOARD_ICONS,
    ];
}

pub fn display_name(_plugin_type: PluginTypeId) -> &'static str {
    "dashboard"
}
