#[cfg(any(
    test,
    feature = "embedded-scheduler",
    feature = "embedded-agent",
    feature = "embedded-ssh-agent",
    feature = "embedded-mqtt"
))]
pub(crate) fn matches_yield_policy(
    policy: uptrakit_service_platform::YieldPolicy,
    app_name: &str,
    local_machine_id: Option<&str>,
    info: &crate::embedded::types::ExternalServiceInfo,
) -> bool {
    match policy {
        uptrakit_service_platform::YieldPolicy::SameServiceSameHost => {
            info.service_app_name.as_deref() == Some(app_name)
                && info.machine_id.as_deref() == local_machine_id
        }
        uptrakit_service_platform::YieldPolicy::SameServiceAnywhere => {
            info.service_app_name.as_deref() == Some(app_name)
        }
        uptrakit_service_platform::YieldPolicy::Never => false,
    }
}
