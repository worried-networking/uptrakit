use uptrakit_notification_plugin_email::EmailNotificationPlugin;
use uptrakit_plugin_releases_github::ReleasesGithubPlugin;
use uptrakit_plugin_infrastructure_core as core;
use uptrakit_plugin_package_manager_apt as apt;

pub fn demo(_plugin: ReleasesGithubPlugin) {
    let _core_alias: Option<core::BatchFetchResult> = None;
    let _apt_alias: Option<apt::AptPlugin> = None;
    let _notification_plugin: Option<EmailNotificationPlugin> = None;
    let _inline: Option<uptrakit_plugin_package_manager_apt::AptPlugin> = None;
}
