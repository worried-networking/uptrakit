use uptrakit_shared_types::PluginTypeId;
use uptrakit_shared_types::PluginTypeId as Id;

pub fn helper_function_items_should_be_rejected() {
    let _display_name = PluginTypeId::display_name;
    let _is_package_manager = Id::is_package_manager;
}
