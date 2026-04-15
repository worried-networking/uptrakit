pub fn production_ok() {}

#[cfg(test)]
use crate::plugin_type_id::plugin_ids::GENERIC_SHELL;

#[cfg(test)]
const TEST_PLUGIN_TYPE: &str = "releases_github";

#[cfg(test)]
fn helper() {
    let _ = GENERIC_SHELL;
    let _ = uptrakit_plugin_infrastructure_core::BatchFetchResult;
}
