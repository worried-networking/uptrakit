use uptrakit_shared_types::PluginTypeId;

struct DisplayCard;

impl DisplayCard {
    fn display_name(&self) -> &str {
        "card"
    }
}

pub fn run() -> &'static str {
    {
        let id: PluginTypeId = todo!();
        let _ = id;
        let _ = '{';
    }

    let id = DisplayCard;
    id.display_name()
}
