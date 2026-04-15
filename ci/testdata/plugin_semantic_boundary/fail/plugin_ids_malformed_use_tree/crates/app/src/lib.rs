use uptrakit_shared_types::{
    plugin_type_id::{
        plugin_ids::{self as ids, GENERIC_SHELL as GS,
        PluginTypeId,
    },
};

pub fn demo() -> PluginTypeId {
    let _ = ids::WEBHOOK;
    GS
}
