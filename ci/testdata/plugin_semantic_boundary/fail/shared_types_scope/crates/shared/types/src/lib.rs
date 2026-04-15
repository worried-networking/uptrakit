pub mod plugin_type_id;

pub use plugin_type_id::{plugin_ids, PluginTypeId};

pub fn shared_types_reference() {
    let _ = plugin_type_id::plugin_ids::GENERIC_SHELL;
}
