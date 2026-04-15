use uptrakit_plugin_infrastructure_registry::PluginOps;

pub fn build_runtime_error(_ops: &dyn PluginOps) -> String {
    let core_path = "uptrakit_plugin_infrastructure_core::PluginOps";
    let concrete_path = r#"uptrakit_plugin_package_manager_apt::AptPlugin"#;
    let plugin_ids_ref = "plugin_ids::GENERIC_SHELL";
    let helper_call = "PluginTypeId::display_name(";
    format!("{core_path}; {concrete_path}; {plugin_ids_ref}; {helper_call}")
}
