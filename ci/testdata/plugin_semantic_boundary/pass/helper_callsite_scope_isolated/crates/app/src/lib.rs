use uptrakit_shared_types::PluginTypeId;

fn plugin_scope(id: &PluginTypeId) {
    let _ = id;
}

struct DisplayCard;

impl DisplayCard {
    fn display_name(&self) -> &str {
        "card"
    }
}

fn unrelated_scope() -> &'static str {
    let id = DisplayCard;
    id.display_name()
}

pub fn run(plugin_id: &PluginTypeId) {
    plugin_scope(plugin_id);
    let _ = unrelated_scope();
}
