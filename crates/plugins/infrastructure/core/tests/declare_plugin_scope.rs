//! Smoke test: declare_plugin! accepts `scope` and `instance_config` arms.
use uptrakit_plugin_infrastructure_core::{
    ConfigModel, InstanceConfigOps, PluginConfig, PluginConfigValidationError, PluginFamily,
    PluginScope, declare_plugin,
};

#[expect(
    dead_code,
    reason = "constructed by declare_plugin! generated code; not directly instantiated in tests"
)]
struct DummyPlugin;
#[derive(Clone, serde::Serialize, serde::Deserialize, Default)]
struct DummyConfig {}
impl PluginConfig for DummyConfig {}

fn dummy_form() -> Vec<uptrakit_plugin_infrastructure_core::FormFieldDescriptor> {
    Vec::new()
}
fn dummy_sample() -> serde_json::Value {
    serde_json::json!({})
}
fn dummy_validate(_v: &serde_json::Value) -> Result<(), PluginConfigValidationError> {
    Ok(())
}

static OPS: InstanceConfigOps = InstanceConfigOps {
    form_schema: dummy_form,
    sample: dummy_sample,
    validate: dummy_validate,
};

declare_plugin!(DummyPlugin, DummyConfig, "test_dummy_instance", {
    display_name: "Dummy Instance Plugin",
    family: PluginFamily::Enhancement,
    config_model: ConfigModel::None,
    scope: PluginScope::Instance,
    instance_config: &OPS,
    roles: [],
});

#[test]
fn descriptor_uses_instance_scope() {
    assert_eq!(DESCRIPTOR.scope, PluginScope::Instance);
    assert!(DESCRIPTOR.instance_config.is_some());
}
