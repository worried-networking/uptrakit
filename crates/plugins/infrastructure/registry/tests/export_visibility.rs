//! Visibility tests for the registry export contract (ST-0015 / DESIGN-0001).

pub fn _accepts_release_fetcher<
    T: uptrakit_plugin_infrastructure_registry::ReleaseFetcher + ?Sized,
>() {
}

#[cfg(feature = "agent-infra")]
pub fn _accepts_infra_action_invoker<
    T: uptrakit_plugin_infrastructure_registry::agent_infra::InfraActionInvoker + ?Sized,
>() {
}

pub fn take_alias<T>(_: uptrakit_plugin_infrastructure_registry::PluginResult<T>) {}

pub fn _accepts_plugin_metadata_ops<
    T: uptrakit_plugin_infrastructure_registry::PluginMetadataOps + ?Sized,
>() {
}

pub fn _accepts_command_executor<
    T: uptrakit_plugin_infrastructure_registry::CommandExecutor + ?Sized,
>() {
}

#[test]
fn block1_infra_core_types_visible() {
    let _: Option<uptrakit_plugin_infrastructure_registry::HostCapabilities> = None;
    let _: Option<uptrakit_plugin_infrastructure_registry::BatchDetectItem> = None;
    let _: Option<uptrakit_plugin_infrastructure_registry::BatchFetchItem> = None;
    let _: Option<uptrakit_plugin_infrastructure_registry::BatchFetchResult> = None;
    let _: Option<uptrakit_plugin_infrastructure_registry::BatchUpdateItem> = None;
    let _: Option<uptrakit_plugin_infrastructure_registry::HostCompatibility> = None;
    let _: Option<uptrakit_plugin_infrastructure_registry::UpdateLifecycleContext> = None;
    let _: Option<uptrakit_plugin_infrastructure_registry::PluginError> = None;
    let _: Option<uptrakit_plugin_infrastructure_registry::PluginFamily> = None;
    let _: Option<uptrakit_plugin_infrastructure_registry::InfraBundle> = None;
    let _: Option<uptrakit_plugin_infrastructure_registry::RoleKey> = None;
    let _ = uptrakit_plugin_infrastructure_registry::construct_host_runtime;
}

#[test]
fn plugin_result_alias_is_exact() {
    let r: std::result::Result<
        u8,
        rootcause::Report<uptrakit_plugin_infrastructure_registry::PluginError>,
    > = Ok(0u8);
    take_alias::<u8>(r);
}

#[test]
fn block2_http_client_types_visible() {
    let _: Option<uptrakit_plugin_infrastructure_registry::PluginHttpClientConfig<'_>> = None;
    let _: Option<uptrakit_plugin_infrastructure_registry::SsrfMode> = None;
    let _ = uptrakit_plugin_infrastructure_registry::build_plugin_http_client;
}

#[cfg(feature = "agent-infra")]
#[test]
fn block3_agent_infra_module_visible() {
    let _: Option<uptrakit_plugin_infrastructure_registry::agent_infra::InfraPluginContext<'_>> =
        None;
    let _: Option<uptrakit_plugin_infrastructure_registry::agent_infra::GuestBootstrapResult> =
        None;
}

#[test]
fn block4_notification_types_visible() {
    let _: Option<uptrakit_plugin_infrastructure_registry::DeliveryMessage> = None;
    let _: Option<uptrakit_plugin_infrastructure_registry::MessageAction> = None;
    let _ = uptrakit_plugin_infrastructure_registry::escape_html;
}

#[test]
fn existing_reexports_still_visible() {
    let _: Option<uptrakit_plugin_infrastructure_registry::PluginCatalog> = None;
    let _: Option<uptrakit_plugin_infrastructure_registry::PluginDescriptor> = None;
    let _: Option<uptrakit_plugin_infrastructure_registry::PluginCapability> = None;
    let _: Option<uptrakit_plugin_infrastructure_registry::ExtensionActionContext<'_>> = None;
}
