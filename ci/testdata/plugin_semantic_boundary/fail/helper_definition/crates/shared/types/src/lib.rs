pub mod plugin_type_id;

pub use plugin_type_id::{plugin_ids, PluginTypeId};

pub fn display_name(id: &PluginTypeId) -> &str {
    id.as_str()
}
