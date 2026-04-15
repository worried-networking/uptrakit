use uptrakit_shared_types::PluginTypeId;

fn from_function() -> PluginTypeId {
    PluginTypeId::from_static("inferred_local_fixture")
}

fn wrap_plugin_type(plugin_type: PluginTypeId) -> PluginTypeId {
    plugin_type
}

fn from_multiline_function(
) -> PluginTypeId
{
    PluginTypeId::from_static("inferred_local_fixture_multiline")
}

fn wrap_multiline_plugin_type(
    plugin_type: PluginTypeId,
)
-> PluginTypeId
{
    plugin_type
}

pub fn inferred_locals_should_be_rejected() {
    let inferred = from_function();
    let _ = inferred.display_name();

    let inferred_nested = wrap_plugin_type(from_function());
    let _ = inferred_nested.is_package_manager();

    let inferred_multiline = from_multiline_function();
    let _ = inferred_multiline.display_name();

    let inferred_multiline_nested = wrap_multiline_plugin_type(from_multiline_function());
    let _ = inferred_multiline_nested.is_package_manager();
}

#[cfg(test)]
mod tests {
    #[test]
    fn inline_tests_are_allowed() {
        assert_eq!("alpha", "alpha");
    }
}
