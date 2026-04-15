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
    pub const PACKAGE_MANAGER_APT: PluginTypeId = PluginTypeId::from_static("package_manager_apt");
    pub const GENERIC_SHELL: PluginTypeId = PluginTypeId::from_static("generic_shell");
    pub const HOOK_SYSTEMD: PluginTypeId = PluginTypeId::from_static("hook_systemd");
    pub const WEBHOOK: PluginTypeId = PluginTypeId::from_static("webhook");
    pub const ENHANCEMENT_DASHBOARD_ICONS: PluginTypeId =
        PluginTypeId::from_static("enhancement_dashboard_icons");
    pub const DISCOVERY_PROXMOX_HELPER_SCRIPTS: PluginTypeId =
        PluginTypeId::from_static("discovery_proxmox_helper_scripts");
    pub const INFRASTRUCTURE_PROXMOX: PluginTypeId =
        PluginTypeId::from_static("infrastructure_proxmox");

    pub const ALL: &[PluginTypeId] = &[
        RELEASES_GITHUB,
        PACKAGE_MANAGER_APT,
        GENERIC_SHELL,
        HOOK_SYSTEMD,
        WEBHOOK,
        ENHANCEMENT_DASHBOARD_ICONS,
        DISCOVERY_PROXMOX_HELPER_SCRIPTS,
        INFRASTRUCTURE_PROXMOX,
    ];

    #[cfg(test)]
    pub const TEST_ONLY: PluginTypeId = PluginTypeId::from_static("__test_only");
}
