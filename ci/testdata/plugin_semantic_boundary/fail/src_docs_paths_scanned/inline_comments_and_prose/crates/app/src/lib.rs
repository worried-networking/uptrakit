pub fn inline_comments_are_not_semantic_usage() {
    let prose = "releases_github appears in prose only";
    let marker = 1; // uptrakit_plugin_infrastructure_core::BatchFetchResult
    let concrete = 2; // uptrakit_plugin_package_manager_apt::AptPlugin
    let module = 3; // plugin_ids::RELEASES_GITHUB
    let helper = 4; // PluginTypeId::is_package_manager
    let _ = (prose, marker, concrete, module, helper); // plugin_type docs-only reference
}
