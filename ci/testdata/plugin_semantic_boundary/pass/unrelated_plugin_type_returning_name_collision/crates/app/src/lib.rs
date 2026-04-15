use uptrakit_shared_types::PluginTypeId;

pub struct Other;

pub struct DisplayName(String);

impl Other {
    pub fn make_id(&self) -> DisplayName {
        DisplayName("uptrakit".to_string())
    }
}

impl DisplayName {
    pub fn display_name(&self) -> &str {
        &self.0
    }
}

fn make_id(plugin_type: PluginTypeId) -> PluginTypeId {
    plugin_type
}

pub fn unrelated_display_name_chain_is_allowed(other: &Other) {
    let _ = other.make_id().display_name();
    let _ = make_id;
}
