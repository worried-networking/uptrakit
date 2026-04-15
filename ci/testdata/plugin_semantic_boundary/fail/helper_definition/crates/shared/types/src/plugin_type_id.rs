use std::borrow::Cow;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PluginTypeId(Cow<'static, str>);

impl PluginTypeId {
    pub const fn from_static(s: &'static str) -> Self {
        Self(Cow::Borrowed(s))
    }

    pub fn is_package_manager(&self) -> bool {
        self.as_str().contains('_')
    }

    pub fn display_name(&self) -> &str {
        self.as_str()
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub mod plugin_ids {
    use super::PluginTypeId;

    pub const GENERIC_SHELL: PluginTypeId = PluginTypeId::from_static("generic_shell");
    pub const ALL: &[PluginTypeId] = &[GENERIC_SHELL];
}
