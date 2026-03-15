use super::*;
use uptrakit_command::NoopCommandExecutor;

fn test_executor() -> Arc<dyn CommandExecutor> {
    Arc::new(NoopCommandExecutor)
}

#[test]
fn parse_known_plugin_types() {
    assert_eq!(
        "releases_github".parse::<PluginType>().ok(),
        Some(PluginType::ReleasesGithub)
    );
    assert_eq!(
        "releases_gitlab".parse::<PluginType>().ok(),
        Some(PluginType::ReleasesGitlab)
    );
    assert_eq!(
        "releases_forgejo".parse::<PluginType>().ok(),
        Some(PluginType::ReleasesForgejo)
    );
    assert_eq!(
        "releases_docker".parse::<PluginType>().ok(),
        Some(PluginType::ReleasesDocker)
    );
    assert_eq!(
        "discovery_proxmox_helper_scripts"
            .parse::<PluginType>()
            .ok(),
        Some(PluginType::DiscoveryProxmoxHelperScripts)
    );
    assert_eq!(
        "package_manager_homebrew".parse::<PluginType>().ok(),
        Some(PluginType::PackageManagerHomebrew)
    );
    assert_eq!(
        "package_manager_apt".parse::<PluginType>().ok(),
        Some(PluginType::PackageManagerApt)
    );
    assert_eq!(
        "generic_shell".parse::<PluginType>().ok(),
        Some(PluginType::GenericShell)
    );
    assert!("unknown".parse::<PluginType>().is_err());
    // Old wire string is no longer a known type
    assert!("docker_registry".parse::<PluginType>().is_err());
}

#[test]
fn validate_valid_github_config() {
    // Empty config is valid — all fields are optional.
    let config = serde_json::json!({});
    assert!(PluginRegistry::validate_config(PluginType::ReleasesGithub, &config).is_ok());
}

#[test]
fn validate_valid_github_config_with_token() {
    let config = serde_json::json!({
        "auth_token": "ghp_test",
        "include_prereleases": false,
        "tag_strip_prefix": "v"
    });
    assert!(PluginRegistry::validate_config(PluginType::ReleasesGithub, &config).is_ok());
}

#[test]
fn validate_invalid_github_config_bad_regex() {
    let config = serde_json::json!({
        "asset_patterns": ["[invalid"]
    });
    assert!(PluginRegistry::validate_config(PluginType::ReleasesGithub, &config).is_err());
}

#[test]
fn validate_valid_docker_config() {
    // Empty config is valid for Docker (no required fields)
    let config = serde_json::json!({});
    assert!(PluginRegistry::validate_config(PluginType::ReleasesDocker, &config).is_ok());
}

#[test]
fn validate_docker_config_old_semver_fields_are_ignored() {
    // Configs stored before the digest-tracking refactor may contain
    // tracking_mode / tag_patterns / page_size — they must be silently ignored.
    let config = serde_json::json!({
        "tracking_mode": "semver_tags",
        "tag_patterns": ["^v[0-9]+"],
        "page_size": 500
    });
    assert!(
        PluginRegistry::validate_config(PluginType::ReleasesDocker, &config).is_ok(),
        "old semver fields should be silently ignored"
    );
}

#[test]
fn validate_proxmox_helper_scripts_config() {
    // PHS config is always `{}`; validation always succeeds.
    let config = serde_json::json!({});
    assert!(
        PluginRegistry::validate_config(PluginType::DiscoveryProxmoxHelperScripts, &config).is_ok()
    );
}

#[test]
fn validate_config_str_valid() {
    let config = serde_json::json!({});
    assert!(PluginRegistry::validate_config_str("releases_github", &config).is_ok());
}

#[test]
fn validate_config_str_unknown_type() {
    let config = serde_json::json!({});
    let result = PluginRegistry::validate_config_str("unknown", &config);
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("unknown plugin type"));
}

#[tokio::test]
async fn create_plugin_github() {
    let config = serde_json::json!({});
    let plugin =
        PluginRegistry::create_plugin(PluginType::ReleasesGithub, &config, test_executor()).await;
    assert!(plugin.is_ok());
}

#[tokio::test]
async fn create_plugin_docker() {
    // Empty config is valid
    let config = serde_json::json!({});
    let plugin =
        PluginRegistry::create_plugin(PluginType::ReleasesDocker, &config, test_executor()).await;
    assert!(plugin.is_ok());
}

#[tokio::test]
async fn create_plugin_for_discovery_docker() {
    let config = serde_json::json!({});
    let plugin = PluginRegistry::create_plugin_for_discovery(
        PluginType::ReleasesDocker,
        &config,
        test_executor(),
    )
    .await;
    assert!(plugin.is_ok());
}

#[tokio::test]
async fn create_plugin_proxmox() {
    // PHS config is always `{}`; extra fields are ignored during deserialization.
    let config = serde_json::json!({});
    let plugin = PluginRegistry::create_plugin(
        PluginType::DiscoveryProxmoxHelperScripts,
        &config,
        test_executor(),
    )
    .await;
    assert!(plugin.is_ok());
}

#[tokio::test]
async fn proxmox_plugin_capabilities() {
    // PHS declares DiscoverLocalSoftware and DetectHostCompatibility.
    // RefreshPackageIndex must not be present.
    let config = serde_json::json!({});
    let plugin = PluginRegistry::create_plugin(
        PluginType::DiscoveryProxmoxHelperScripts,
        &config,
        test_executor(),
    )
    .await
    .expect("create");
    assert!(plugin.has_capability(
        uptrakit_plugin_infrastructure_core::PluginCapability::DiscoverLocalSoftware
    ));
    assert!(!plugin.has_capability(
        uptrakit_plugin_infrastructure_core::PluginCapability::RefreshPackageIndex
    ));
}

#[test]
fn mask_config_secrets_proxmox_is_noop() {
    // PHS has no secret fields; masking returns an equivalent empty object.
    let config = serde_json::json!({});
    let masked =
        PluginRegistry::mask_config_secrets(PluginType::DiscoveryProxmoxHelperScripts, &config);
    assert_eq!(masked, serde_json::json!({}));
}

#[test]
fn restore_config_secrets_proxmox_is_noop() {
    // PHS has no secret fields; restoring is a no-op.
    let mut incoming = serde_json::json!({});
    let existing = serde_json::json!({});
    PluginRegistry::restore_config_secrets(
        PluginType::DiscoveryProxmoxHelperScripts,
        &mut incoming,
        &existing,
    );
    assert_eq!(incoming, serde_json::json!({}));
}

#[tokio::test]
async fn create_plugin_homebrew() {
    let config = serde_json::json!({});
    let plugin =
        PluginRegistry::create_plugin(PluginType::PackageManagerHomebrew, &config, test_executor())
            .await;
    assert!(plugin.is_ok());
}

#[tokio::test]
async fn create_plugin_homebrew_cask() {
    let config = serde_json::json!({"package_type": "cask"});
    let plugin =
        PluginRegistry::create_plugin(PluginType::PackageManagerHomebrew, &config, test_executor())
            .await;
    assert!(plugin.is_ok());
}

#[test]
fn validate_homebrew_config() {
    let config = serde_json::json!({});
    assert!(PluginRegistry::validate_config(PluginType::PackageManagerHomebrew, &config).is_ok());
}

#[test]
fn validate_homebrew_config_cask() {
    let config = serde_json::json!({"package_type": "cask"});
    assert!(PluginRegistry::validate_config(PluginType::PackageManagerHomebrew, &config).is_ok());
}

#[test]
fn validate_homebrew_config_invalid_package_type() {
    let config = serde_json::json!({"package_type": "invalid"});
    assert!(PluginRegistry::validate_config(PluginType::PackageManagerHomebrew, &config).is_err());
}

#[tokio::test]
async fn homebrew_plugin_capabilities() {
    let config = serde_json::json!({});
    let plugin =
        PluginRegistry::create_plugin(PluginType::PackageManagerHomebrew, &config, test_executor())
            .await
            .unwrap();
    assert!(plugin.has_capability(
        uptrakit_plugin_infrastructure_core::PluginCapability::DiscoverLocalSoftware
    ));
    assert!(plugin.has_capability(
        uptrakit_plugin_infrastructure_core::PluginCapability::RefreshPackageIndex
    ));
}

#[tokio::test]
async fn docker_plugin_capabilities() {
    let config = serde_json::json!({});
    let plugin =
        PluginRegistry::create_plugin(PluginType::ReleasesDocker, &config, test_executor())
            .await
            .unwrap();
    assert!(plugin.has_capability(
        uptrakit_plugin_infrastructure_core::PluginCapability::DiscoverLocalSoftware
    ));
    assert!(!plugin.has_capability(
        uptrakit_plugin_infrastructure_core::PluginCapability::RefreshPackageIndex
    ));
}

#[test]
fn discovery_plugins_includes_docker() {
    let types = PluginRegistry::discovery_plugins();
    assert!(
        types.contains(&PluginType::ReleasesDocker),
        "Docker should be in discovery_plugins()"
    );
}

#[tokio::test]
async fn all_required_sudo_commands_includes_apt() {
    let entries = PluginRegistry::all_required_sudo_commands().await;
    let apt_entry = entries
        .iter()
        .find(|(pt, _)| *pt == PluginType::PackageManagerApt)
        .expect("Apt should have sudo command entries");
    assert!(!apt_entry.1.is_empty());
    assert_eq!(apt_entry.1[0].command, "apt-get");
}

#[tokio::test]
async fn all_required_sudo_commands_no_duplicates_per_plugin() {
    let entries = PluginRegistry::all_required_sudo_commands().await;
    // All entries in results should have non-empty command lists
    for (pt, cmds) in &entries {
        assert!(
            !cmds.is_empty(),
            "plugin {pt} has empty sudo command list but was included"
        );
    }
}

// ── compatible_sudo_commands_for_host tests ───────────────────────────

#[tokio::test]
async fn compatible_sudo_commands_for_host_returns_valid_entries() {
    let executor = test_executor();
    let entries = PluginRegistry::compatible_sudo_commands_for_host(executor).await;
    // Every included plugin must have a non-empty command list.
    for (pt, cmds) in &entries {
        assert!(
            !cmds.is_empty(),
            "plugin {pt} returned an empty sudo command list but was included"
        );
    }
}

#[tokio::test]
async fn compatible_sudo_commands_for_host_excludes_phs_on_non_phs_host() {
    // On any host that lacks /usr/bin/update the PHS plugin must not be
    // included — its helper scripts must not be installed on non-Proxmox
    // machines (e.g. Flatcar Linux with a read-only /usr/local/bin).
    let executor = test_executor();
    let entries = PluginRegistry::compatible_sudo_commands_for_host(executor).await;
    let phs_entry = entries
        .iter()
        .find(|(pt, _)| *pt == PluginType::DiscoveryProxmoxHelperScripts);
    assert!(
        phs_entry.is_none(),
        "PHS must not be included on a non-PHS host (no /usr/bin/update found)"
    );
}

#[tokio::test]
async fn boxed_plugin_preserves_type() {
    let github_config = serde_json::json!({});
    let github =
        PluginRegistry::create_plugin(PluginType::ReleasesGithub, &github_config, test_executor())
            .await
            .expect("create github");
    assert_eq!(github.plugin_type_id(), PluginType::ReleasesGithub.as_str());

    let docker_config = serde_json::json!({});
    let docker =
        PluginRegistry::create_plugin(PluginType::ReleasesDocker, &docker_config, test_executor())
            .await
            .expect("create docker");
    assert_eq!(docker.plugin_type_id(), PluginType::ReleasesDocker.as_str());

    let proxmox_config = serde_json::json!({});
    let proxmox = PluginRegistry::create_plugin(
        PluginType::DiscoveryProxmoxHelperScripts,
        &proxmox_config,
        test_executor(),
    )
    .await
    .expect("create proxmox");
    assert_eq!(
        proxmox.plugin_type_id(),
        PluginType::DiscoveryProxmoxHelperScripts.as_str()
    );

    let homebrew_config = serde_json::json!({});
    let homebrew = PluginRegistry::create_plugin(
        PluginType::PackageManagerHomebrew,
        &homebrew_config,
        test_executor(),
    )
    .await
    .expect("create homebrew");
    assert_eq!(
        homebrew.plugin_type_id(),
        PluginType::PackageManagerHomebrew.as_str()
    );

    let apt_config = serde_json::json!({});
    let apt =
        PluginRegistry::create_plugin(PluginType::PackageManagerApt, &apt_config, test_executor())
            .await
            .expect("create apt");
    assert_eq!(apt.plugin_type_id(), PluginType::PackageManagerApt.as_str());
}

#[test]
fn mask_config_secrets_homebrew() {
    let config = serde_json::json!({"package_type": "formula"});
    let masked = PluginRegistry::mask_config_secrets(PluginType::PackageManagerHomebrew, &config);
    assert_eq!(masked, config);
}

#[test]
fn mask_config_secrets_github() {
    let config = serde_json::json!({
        "auth_token": "ghp_secret"
    });
    let masked = PluginRegistry::mask_config_secrets(PluginType::ReleasesGithub, &config);
    assert_eq!(masked["auth_token"], "***");
}

#[test]
fn mask_config_secrets_github_always_shows_field() {
    // Even with no auth_token in input, masked output always includes the field.
    let config = serde_json::json!({});
    let masked = PluginRegistry::mask_config_secrets(PluginType::ReleasesGithub, &config);
    assert_eq!(masked["auth_token"], "***");
}

#[test]
fn restore_config_secrets_github() {
    let mut incoming = serde_json::json!({
        "auth_token": "***"
    });
    let existing = serde_json::json!({
        "auth_token": "ghp_real_token"
    });
    PluginRegistry::restore_config_secrets(PluginType::ReleasesGithub, &mut incoming, &existing);
    assert_eq!(incoming["auth_token"], "ghp_real_token");
}

#[tokio::test]
async fn create_plugin_apt() {
    let config = serde_json::json!({});
    let plugin =
        PluginRegistry::create_plugin(PluginType::PackageManagerApt, &config, test_executor())
            .await;
    assert!(plugin.is_ok());
}

#[tokio::test]
async fn create_plugin_apt_all_filter() {
    let config = serde_json::json!({"discovery_filter": "all"});
    let plugin =
        PluginRegistry::create_plugin(PluginType::PackageManagerApt, &config, test_executor())
            .await;
    assert!(plugin.is_ok());
}

#[test]
fn validate_apt_config() {
    let config = serde_json::json!({});
    assert!(PluginRegistry::validate_config(PluginType::PackageManagerApt, &config).is_ok());
}

#[test]
fn validate_apt_config_invalid_filter_fails() {
    let config = serde_json::json!({"discovery_filter": "unknown"});
    assert!(PluginRegistry::validate_config(PluginType::PackageManagerApt, &config).is_err());
}

#[tokio::test]
async fn apt_plugin_capabilities() {
    let config = serde_json::json!({});
    let plugin =
        PluginRegistry::create_plugin(PluginType::PackageManagerApt, &config, test_executor())
            .await
            .unwrap();
    assert!(plugin.has_capability(
        uptrakit_plugin_infrastructure_core::PluginCapability::DiscoverLocalSoftware
    ));
    assert!(plugin.has_capability(
        uptrakit_plugin_infrastructure_core::PluginCapability::RefreshPackageIndex
    ));
}

#[test]
fn mask_config_secrets_apt() {
    let config = serde_json::json!({"discovery_filter": "manual"});
    let masked = PluginRegistry::mask_config_secrets(PluginType::PackageManagerApt, &config);
    assert_eq!(masked, config);
}

#[test]
fn validate_package_identifier_apt_valid() {
    assert!(
        PluginRegistry::validate_package_identifier(PluginType::PackageManagerApt, "nginx").is_ok()
    );
}

#[test]
fn validate_package_identifier_apt_uppercase_fails() {
    assert!(
        PluginRegistry::validate_package_identifier(PluginType::PackageManagerApt, "Nginx")
            .is_err()
    );
}

// ── Shell plugin tests ────────────────────────────────────────────────

#[tokio::test]
async fn create_plugin_shell_version_only() {
    let config = serde_json::json!({"version_command": "myapp --version"});
    let plugin =
        PluginRegistry::create_plugin(PluginType::GenericShell, &config, test_executor()).await;
    assert!(plugin.is_ok());
}

#[tokio::test]
async fn create_plugin_shell_update_only() {
    let config = serde_json::json!({"update_command": "apt-get install -y myapp"});
    let plugin =
        PluginRegistry::create_plugin(PluginType::GenericShell, &config, test_executor()).await;
    assert!(plugin.is_ok());
}

#[tokio::test]
async fn create_plugin_shell_both() {
    let config = serde_json::json!({
        "version_command": "myapp --version",
        "update_command": "apt-get install -y myapp"
    });
    let plugin =
        PluginRegistry::create_plugin(PluginType::GenericShell, &config, test_executor()).await;
    assert!(plugin.is_ok());
}

#[test]
fn validate_config_shell_both_none_fails() {
    // Empty config — both commands absent — must fail validation.
    let config = serde_json::json!({});
    assert!(PluginRegistry::validate_config(PluginType::GenericShell, &config).is_err());
}

// ── validate_package_identifier GitHub tests ──────────────────────────

#[test]
fn validate_package_identifier_github_valid() {
    assert!(
        PluginRegistry::validate_package_identifier(
            PluginType::ReleasesGithub,
            "octocat/hello-world"
        )
        .is_ok()
    );
}

#[test]
fn validate_package_identifier_github_no_slash_fails() {
    assert!(
        PluginRegistry::validate_package_identifier(PluginType::ReleasesGithub, "octocat").is_err()
    );
}

#[test]
fn validate_package_identifier_github_traversal_fails() {
    assert!(
        PluginRegistry::validate_package_identifier(PluginType::ReleasesGithub, "octocat/../evil")
            .is_err()
    );
}

#[test]
fn validate_package_identifier_github_empty_repo_fails() {
    assert!(
        PluginRegistry::validate_package_identifier(PluginType::ReleasesGithub, "octocat/")
            .is_err()
    );
}

// ── validate_package_identifier GitLab tests ──────────────────────────

#[test]
fn validate_package_identifier_gitlab_simple_valid() {
    assert!(
        PluginRegistry::validate_package_identifier(PluginType::ReleasesGitlab, "owner/project")
            .is_ok()
    );
}

#[test]
fn validate_package_identifier_gitlab_nested_namespace_valid() {
    assert!(
        PluginRegistry::validate_package_identifier(
            PluginType::ReleasesGitlab,
            "group/subgroup/project"
        )
        .is_ok()
    );
}

#[test]
fn validate_package_identifier_gitlab_no_slash_fails() {
    assert!(
        PluginRegistry::validate_package_identifier(PluginType::ReleasesGitlab, "project").is_err()
    );
}

#[test]
fn validate_package_identifier_gitlab_traversal_fails() {
    assert!(
        PluginRegistry::validate_package_identifier(PluginType::ReleasesGitlab, "../evil/project")
            .is_err()
    );
}

#[test]
fn validate_package_identifier_gitlab_empty_component_fails() {
    assert!(
        PluginRegistry::validate_package_identifier(PluginType::ReleasesGitlab, "owner//project")
            .is_err()
    );
}

// ── validate_package_identifier Forgejo tests ─────────────────────────

#[test]
fn validate_package_identifier_forgejo_valid() {
    assert!(
        PluginRegistry::validate_package_identifier(PluginType::ReleasesForgejo, "owner/repo")
            .is_ok()
    );
}

#[test]
fn validate_package_identifier_forgejo_no_slash_fails() {
    assert!(
        PluginRegistry::validate_package_identifier(PluginType::ReleasesForgejo, "owner").is_err()
    );
}

#[test]
fn validate_package_identifier_forgejo_traversal_fails() {
    assert!(
        PluginRegistry::validate_package_identifier(PluginType::ReleasesForgejo, "owner/../evil")
            .is_err()
    );
}

#[test]
fn validate_package_identifier_forgejo_empty_repo_fails() {
    assert!(
        PluginRegistry::validate_package_identifier(PluginType::ReleasesForgejo, "owner/").is_err()
    );
}

// ── PluginType::Other(String) behaviour ──────────────────────────────

/// `Other(String)` received from a newer server must fail gracefully at
/// the registry level (unknown type) rather than causing a deserialization
/// panic or silent data loss.
#[tokio::test]
async fn create_plugin_other_returns_unknown_type_error() {
    let config = serde_json::json!({});
    let Err(err) = PluginRegistry::create_plugin(
        PluginType::Other("winget".to_string()),
        &config,
        test_executor(),
    )
    .await
    else {
        panic!("expected Err for Other plugin type");
    };
    assert!(err.to_string().contains("unknown plugin type"));
}

#[test]
fn validate_config_other_returns_unknown_type_error() {
    let config = serde_json::json!({});
    let result = PluginRegistry::validate_config(PluginType::Other("winget".to_string()), &config);
    assert!(result.is_err());
}

/// `mask_config_secrets` for an `Other` plugin type returns the config
/// unchanged (no masking possible for an unknown plugin).
#[test]
fn mask_config_secrets_other_returns_config_unchanged() {
    let config = serde_json::json!({"token": "secret", "repo": "something"});
    let result =
        PluginRegistry::mask_config_secrets(PluginType::Other("winget".to_string()), &config);
    assert_eq!(result, config);
}

// ── validate_package_identifier ───────────────────────────────────────

/// `Other` always returns `Ok(())`.
#[test]
fn validate_package_identifier_other_is_permissive() {
    assert!(
        PluginRegistry::validate_package_identifier(
            PluginType::Other("flatpak".to_string()),
            "org.example.App"
        )
        .is_ok()
    );
}

#[test]
fn validate_package_identifier_docker_valid() {
    assert!(
        PluginRegistry::validate_package_identifier(PluginType::ReleasesDocker, "nginx").is_ok()
    );
    assert!(
        PluginRegistry::validate_package_identifier(
            PluginType::ReleasesDocker,
            "ghcr.io/owner/app:latest"
        )
        .is_ok()
    );
    assert!(
        PluginRegistry::validate_package_identifier(PluginType::ReleasesDocker, "myuser/app:v2")
            .is_ok()
    );
}

#[test]
fn validate_package_identifier_docker_invalid() {
    assert!(PluginRegistry::validate_package_identifier(PluginType::ReleasesDocker, "").is_err());
    assert!(
        PluginRegistry::validate_package_identifier(PluginType::ReleasesDocker, "nginx latest")
            .is_err()
    );
    assert!(
        PluginRegistry::validate_package_identifier(PluginType::ReleasesDocker, "ghcr.io//app")
            .is_err()
    );
}

// ── capabilities_for / capabilities_for_str ───────────────────────────

#[test]
fn capabilities_for_docker_includes_discover() {
    let caps = PluginRegistry::capabilities_for(PluginType::ReleasesDocker);
    assert!(
        caps.contains(
            &uptrakit_plugin_infrastructure_core::PluginCapability::DiscoverLocalSoftware
        ),
        "Docker plugin should declare DiscoverLocalSoftware"
    );
}

#[test]
fn capabilities_for_github_is_empty() {
    let caps = PluginRegistry::capabilities_for(PluginType::ReleasesGithub);
    assert!(
        !caps.contains(
            &uptrakit_plugin_infrastructure_core::PluginCapability::DiscoverLocalSoftware
        ),
        "GitHub releases plugin must not declare DiscoverLocalSoftware"
    );
}

#[test]
fn capabilities_for_str_docker_includes_discover() {
    let caps = PluginRegistry::capabilities_for_str("releases_docker");
    assert!(
        caps.contains(
            &uptrakit_plugin_infrastructure_core::PluginCapability::DiscoverLocalSoftware
        ),
        "releases_docker should declare DiscoverLocalSoftware via string lookup"
    );
}

#[test]
fn capabilities_for_str_github_has_no_discover() {
    let caps = PluginRegistry::capabilities_for_str("releases_github");
    assert!(
        !caps.contains(
            &uptrakit_plugin_infrastructure_core::PluginCapability::DiscoverLocalSoftware
        ),
        "releases_github must not declare DiscoverLocalSoftware"
    );
}

#[test]
fn capabilities_for_str_unknown_returns_empty() {
    let caps = PluginRegistry::capabilities_for_str("unknown_type");
    assert!(
        caps.is_empty(),
        "Unknown plugin type should return an empty capabilities vec"
    );
}

#[test]
fn capabilities_for_str_generic_shell_has_no_discover() {
    let caps = PluginRegistry::capabilities_for_str("generic_shell");
    assert!(
        !caps.contains(
            &uptrakit_plugin_infrastructure_core::PluginCapability::DiscoverLocalSoftware
        ),
        "generic_shell must not declare DiscoverLocalSoftware"
    );
}

// ── npm plugin tests ──────────────────────────────────────────────────────

#[tokio::test]
async fn create_plugin_npm() {
    let config = serde_json::json!({});
    let plugin =
        PluginRegistry::create_plugin(PluginType::PackageManagerNpm, &config, test_executor())
            .await;
    assert!(plugin.is_ok());
}

#[tokio::test]
async fn create_plugin_npm_with_prereleases() {
    let config = serde_json::json!({"include_prereleases": true});
    let plugin =
        PluginRegistry::create_plugin(PluginType::PackageManagerNpm, &config, test_executor())
            .await;
    assert!(plugin.is_ok());
}

#[test]
fn validate_npm_config() {
    let config = serde_json::json!({});
    assert!(PluginRegistry::validate_config(PluginType::PackageManagerNpm, &config).is_ok());
}

#[tokio::test]
async fn npm_plugin_capabilities() {
    let config = serde_json::json!({});
    let plugin =
        PluginRegistry::create_plugin(PluginType::PackageManagerNpm, &config, test_executor())
            .await
            .unwrap();
    assert!(plugin.has_capability(
        uptrakit_plugin_infrastructure_core::PluginCapability::DiscoverLocalSoftware
    ));
    assert!(plugin.has_capability(
        uptrakit_plugin_infrastructure_core::PluginCapability::ControllerSideFetchReleases
    ));
    assert!(!plugin.has_capability(
        uptrakit_plugin_infrastructure_core::PluginCapability::RefreshPackageIndex
    ));
}

#[test]
fn mask_config_secrets_npm_is_noop() {
    let config = serde_json::json!({"include_prereleases": false});
    let masked = PluginRegistry::mask_config_secrets(PluginType::PackageManagerNpm, &config);
    assert_eq!(masked, config);
}

#[test]
fn validate_package_identifier_npm_plain_valid() {
    assert!(
        PluginRegistry::validate_package_identifier(PluginType::PackageManagerNpm, "n8n").is_ok()
    );
}

#[test]
fn validate_package_identifier_npm_scoped_valid() {
    assert!(
        PluginRegistry::validate_package_identifier(PluginType::PackageManagerNpm, "@angular/cli")
            .is_ok()
    );
}

#[test]
fn validate_package_identifier_npm_uppercase_fails() {
    assert!(
        PluginRegistry::validate_package_identifier(PluginType::PackageManagerNpm, "MyPackage")
            .is_err()
    );
}

#[tokio::test]
async fn all_required_sudo_commands_includes_npm() {
    let entries = PluginRegistry::all_required_sudo_commands().await;
    let npm_entry = entries
        .iter()
        .find(|(pt, _)| *pt == PluginType::PackageManagerNpm)
        .expect("npm should have sudo command entries");
    assert!(!npm_entry.1.is_empty());
    assert_eq!(npm_entry.1[0].command, "npm");
}

#[test]
fn discovery_plugins_includes_npm() {
    let types = PluginRegistry::discovery_plugins();
    assert!(
        types.contains(&PluginType::PackageManagerNpm),
        "npm should be in discovery_plugins()"
    );
}

#[test]
fn discovery_plugins_includes_mas() {
    let types = PluginRegistry::discovery_plugins();
    assert!(
        types.contains(&PluginType::PackageManagerMas),
        "Mac App Store plugin should be in discovery_plugins()"
    );
}

// ── Snap plugin tests ─────────────────────────────────────────────────────

#[tokio::test]
async fn create_plugin_snap() {
    let config = serde_json::json!({});
    let plugin =
        PluginRegistry::create_plugin(PluginType::PackageManagerSnap, &config, test_executor())
            .await;
    assert!(plugin.is_ok());
}

#[tokio::test]
async fn create_plugin_snap_with_channel() {
    let config = serde_json::json!({"channel": "latest/stable"});
    let plugin =
        PluginRegistry::create_plugin(PluginType::PackageManagerSnap, &config, test_executor())
            .await;
    assert!(plugin.is_ok());
}

#[test]
fn validate_snap_config() {
    let config = serde_json::json!({});
    assert!(PluginRegistry::validate_config(PluginType::PackageManagerSnap, &config).is_ok());
}

#[test]
fn validate_snap_config_invalid_channel_fails() {
    let config = serde_json::json!({"channel": "latest/nightly"});
    assert!(PluginRegistry::validate_config(PluginType::PackageManagerSnap, &config).is_err());
}

#[tokio::test]
async fn snap_plugin_capabilities() {
    let config = serde_json::json!({});
    let plugin =
        PluginRegistry::create_plugin(PluginType::PackageManagerSnap, &config, test_executor())
            .await
            .unwrap();
    assert!(plugin.has_capability(
        uptrakit_plugin_infrastructure_core::PluginCapability::DiscoverLocalSoftware
    ));
    assert!(plugin.has_capability(
        uptrakit_plugin_infrastructure_core::PluginCapability::DetectHostCompatibility
    ));
    // Snap does not need RefreshPackageIndex — snapd manages its own cache.
    assert!(!plugin.has_capability(
        uptrakit_plugin_infrastructure_core::PluginCapability::RefreshPackageIndex
    ));
}

#[test]
fn mask_config_secrets_snap_is_noop() {
    let config = serde_json::json!({"channel": "latest/stable"});
    let masked = PluginRegistry::mask_config_secrets(PluginType::PackageManagerSnap, &config);
    assert_eq!(masked, config);
}

#[test]
fn validate_package_identifier_snap_valid() {
    assert!(
        PluginRegistry::validate_package_identifier(PluginType::PackageManagerSnap, "vlc").is_ok()
    );
    assert!(
        PluginRegistry::validate_package_identifier(PluginType::PackageManagerSnap, "hello-world")
            .is_ok()
    );
}

#[test]
fn validate_package_identifier_snap_hyphen_start_fails() {
    assert!(
        PluginRegistry::validate_package_identifier(PluginType::PackageManagerSnap, "-invalid")
            .is_err()
    );
}

#[test]
fn validate_package_identifier_snap_uppercase_fails() {
    assert!(
        PluginRegistry::validate_package_identifier(PluginType::PackageManagerSnap, "VLC").is_err()
    );
}

#[tokio::test]
async fn snap_has_sudo_command_entry() {
    let entries = PluginRegistry::all_required_sudo_commands().await;
    let snap_entry = entries
        .iter()
        .find(|(pt, _)| *pt == PluginType::PackageManagerSnap)
        .expect("Snap should have sudo command entries");
    assert!(!snap_entry.1.is_empty());
    assert_eq!(snap_entry.1[0].command, "snap");
}

#[test]
fn discovery_plugins_includes_snap() {
    let types = PluginRegistry::discovery_plugins();
    assert!(
        types.contains(&PluginType::PackageManagerSnap),
        "Snap plugin should be in discovery_plugins()"
    );
}

// ── Cargo plugin tests ────────────────────────────────────────────────────

#[tokio::test]
async fn create_plugin_cargo() {
    let config = serde_json::json!({});
    let plugin =
        PluginRegistry::create_plugin(PluginType::PackageManagerCargo, &config, test_executor())
            .await;
    assert!(plugin.is_ok());
}

#[tokio::test]
async fn create_plugin_cargo_with_include_prereleases() {
    let config = serde_json::json!({"include_prereleases": true});
    let plugin =
        PluginRegistry::create_plugin(PluginType::PackageManagerCargo, &config, test_executor())
            .await;
    assert!(plugin.is_ok());
}

#[test]
fn validate_cargo_config_default() {
    let config = serde_json::json!({});
    assert!(PluginRegistry::validate_config(PluginType::PackageManagerCargo, &config).is_ok());
}

#[test]
fn validate_cargo_config_empty_registry_url_fails() {
    let config = serde_json::json!({"registry_url": ""});
    assert!(PluginRegistry::validate_config(PluginType::PackageManagerCargo, &config).is_err());
}

#[tokio::test]
async fn cargo_plugin_capabilities() {
    let config = serde_json::json!({});
    let plugin =
        PluginRegistry::create_plugin(PluginType::PackageManagerCargo, &config, test_executor())
            .await
            .unwrap();
    assert!(plugin.has_capability(
        uptrakit_plugin_infrastructure_core::PluginCapability::DiscoverLocalSoftware
    ));
    assert!(plugin.has_capability(
        uptrakit_plugin_infrastructure_core::PluginCapability::DetectHostCompatibility
    ));
    assert!(plugin.has_capability(
        uptrakit_plugin_infrastructure_core::PluginCapability::ControllerSideFetchReleases
    ));
}

#[test]
fn mask_config_secrets_cargo_is_noop() {
    let config = serde_json::json!({"include_prereleases": true, "use_locked": true});
    let masked = PluginRegistry::mask_config_secrets(PluginType::PackageManagerCargo, &config);
    assert_eq!(masked, config);
}

#[test]
fn validate_package_identifier_cargo_valid() {
    assert!(
        PluginRegistry::validate_package_identifier(PluginType::PackageManagerCargo, "ripgrep")
            .is_ok()
    );
    assert!(
        PluginRegistry::validate_package_identifier(
            PluginType::PackageManagerCargo,
            "cargo-nextest"
        )
        .is_ok()
    );
    assert!(
        PluginRegistry::validate_package_identifier(PluginType::PackageManagerCargo, "_helper")
            .is_ok()
    );
}

#[test]
fn validate_package_identifier_cargo_leading_digit_fails() {
    assert!(
        PluginRegistry::validate_package_identifier(PluginType::PackageManagerCargo, "1crate")
            .is_err()
    );
}

#[test]
fn validate_package_identifier_cargo_dot_fails() {
    assert!(
        PluginRegistry::validate_package_identifier(PluginType::PackageManagerCargo, "my.crate")
            .is_err()
    );
}

#[tokio::test]
async fn cargo_has_no_sudo_commands() {
    let entries = PluginRegistry::all_required_sudo_commands().await;
    let cargo_entry = entries
        .iter()
        .find(|(pt, _)| *pt == PluginType::PackageManagerCargo);
    // Cargo install requires no sudo — either no entry or an empty entry list.
    assert!(
        cargo_entry.is_none() || cargo_entry.is_some_and(|(_, cmds)| cmds.is_empty()),
        "Cargo plugin should not require any sudo commands"
    );
}

#[test]
fn discovery_plugins_includes_cargo() {
    let types = PluginRegistry::discovery_plugins();
    assert!(
        types.contains(&PluginType::PackageManagerCargo),
        "Cargo plugin should be in discovery_plugins()"
    );
}
