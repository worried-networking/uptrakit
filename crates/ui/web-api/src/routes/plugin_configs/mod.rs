mod audit;
mod batch;
mod command_safety;
mod crud;
mod discover;
mod test_action;

pub use batch::{__path_batch_plugin_configs, batch_plugin_configs};
pub use crud::ListPluginConfigsParams;
pub(crate) use crud::plugin_field_to_api_field;
pub use crud::{
    __path_create_plugin_config, __path_delete_plugin_config, __path_get_plugin_config,
    __path_list_plugin_configs, __path_list_plugin_types, __path_update_plugin_config,
    create_plugin_config, delete_plugin_config, get_plugin_config, list_plugin_configs,
    list_plugin_types, update_plugin_config,
};
pub use discover::{__path_discover_plugin_config, discover_plugin_config};
pub use test_action::{__path_test_plugin_config, test_plugin_config};
pub use uptrakit_web_api_types::batch_actions::{
    BatchActionFailure, BatchActionRequest, BatchActionResponse, BatchActionSuccess,
};
pub use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};
pub use uptrakit_web_api_types::plugin_configs::{
    CreatePluginConfigRequest, PluginConfigResponse, PluginTypeInfo, UpdatePluginConfigRequest,
};

#[cfg(test)]
mod tests;
