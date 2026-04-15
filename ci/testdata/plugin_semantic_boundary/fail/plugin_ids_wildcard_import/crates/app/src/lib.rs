use uptrakit_shared_types::plugin_type_id::plugin_ids::*;
use uptrakit_shared_types::plugin_type_id::{plugin_ids::*, PluginTypeId};

pub fn demo() {
    let _ = GENERIC_SHELL;
    let _ = WEBHOOK;
    let _ = PACKAGE_MANAGER_APT;
    let _ = [GENERIC_SHELL, WEBHOOK, PACKAGE_MANAGER_APT];
    let _ = GENERIC_SHELL;
    let _ = PluginTypeId::from_static("wildcard_fixture");
}
