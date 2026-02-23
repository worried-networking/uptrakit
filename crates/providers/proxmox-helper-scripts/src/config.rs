use rootcause::prelude::*;
use serde::{Deserialize, Serialize};
use uptrakit_provider_core::{ProviderError, SecretMasking, SecretString};

/// Sentinel value used to indicate a masked secret in API responses.
const SECRET_MASK: &str = "***";

/// Optional GitHub release source configuration for upstream version detection.
///
/// When present, the PHS provider delegates `fetch_releases()` to an internal
/// `GitHubProvider` instance. Since different PHS apps have different upstream
/// GitHub repos, this is typically provided via per-item `config_override`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitHubReleaseSource {
    /// GitHub repository owner (user or organization).
    pub owner: String,
    /// GitHub repository name.
    pub repo: String,
    /// Optional personal access token for authentication (increases rate limits).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<SecretString>,
    /// Prefix to strip from tags when extracting version strings (e.g. "v").
    #[serde(default = "default_tag_strip_prefix")]
    pub tag_strip_prefix: String,
    /// Whether to include pre-releases in the results.
    #[serde(default)]
    pub include_prereleases: bool,
}

fn default_tag_strip_prefix() -> String {
    "v".to_string()
}

impl GitHubReleaseSource {
    /// Validate the GitHub release source fields.
    pub fn validate(&self) -> uptrakit_provider_core::Result<()> {
        if self.owner.is_empty() {
            bail!(ProviderError::Configuration(
                "github.owner must not be empty".to_string()
            ));
        }
        if self.repo.is_empty() {
            bail!(ProviderError::Configuration(
                "github.repo must not be empty".to_string()
            ));
        }
        if self.owner.contains('/') || self.owner.contains("..") {
            bail!(ProviderError::Configuration(
                "github.owner must not contain '/' or '..'".to_string()
            ));
        }
        if self.repo.contains('/') || self.repo.contains("..") {
            bail!(ProviderError::Configuration(
                "github.repo must not contain '/' or '..'".to_string()
            ));
        }
        Ok(())
    }
}

/// Configuration for the Proxmox Helper Scripts provider.
///
/// `script_url` is optional for deserialization so that an empty `{}` config
/// can be used for autodiscovery. The field defaults to an empty string when
/// absent. `validate()` rejects an empty `script_url` only in version-check
/// context; discovery proceeds with `script_url = ""`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProxmoxHelperScriptsConfig {
    /// URL of the Proxmox helper script to execute for updates.
    ///
    /// Optional at deserialization time; required for update execution.
    /// Defaults to an empty string when the field is absent from JSON.
    #[serde(default)]
    pub script_url: String,
    /// Optional GitHub release source for upstream version detection.
    ///
    /// When present, the provider gains `RefreshPackageIndex` capability and
    /// delegates `fetch_releases()` to an internal GitHub provider instance.
    /// Typically provided via per-item `config_override` since different PHS
    /// apps have different upstream GitHub repos.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<GitHubReleaseSource>,
}

impl SecretMasking for ProxmoxHelperScriptsConfig {
    fn with_secrets_masked(mut self) -> Self {
        if let Some(ref mut gh) = self.github {
            gh.auth_token = Some(SecretString::new(SECRET_MASK.to_string()));
        }
        self
    }

    fn restore_secrets_from(&mut self, existing: &Self) {
        if let (Some(gh), Some(existing_gh)) = (&mut self.github, &existing.github)
            && let Some(ref token) = gh.auth_token
            && token.expose_secret() == SECRET_MASK
        {
            gh.auth_token = existing_gh.auth_token.clone();
        }
    }
}

impl ProxmoxHelperScriptsConfig {
    /// Validate the configuration for version-check / update-execution context.
    ///
    /// Rejects an empty `script_url` because it is required for update execution.
    /// This validation must NOT be called during discovery, where an empty
    /// `script_url` is acceptable.
    pub fn validate(&self) -> uptrakit_provider_core::Result<()> {
        if self.script_url.is_empty() {
            bail!(ProviderError::MissingConfig("script_url".to_string()));
        }
        if let Some(ref gh) = self.github {
            gh.validate()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_config() {
        let config = ProxmoxHelperScriptsConfig {
            script_url: "https://example.com/script.sh".to_string(),
            github: None,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn empty_script_url_fails_validate() {
        let config = ProxmoxHelperScriptsConfig {
            script_url: String::new(),
            github: None,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn deserialize_empty_object_succeeds() {
        // Discovery sends {} as the default config — must not fail to deserialize.
        let config: ProxmoxHelperScriptsConfig =
            serde_json::from_str("{}").expect("deserialize empty config");
        assert!(config.script_url.is_empty());
        assert!(config.github.is_none());
    }

    #[test]
    fn deserialization_without_github() {
        let json = r#"{"script_url":"https://example.com/update.sh"}"#;
        let config: ProxmoxHelperScriptsConfig = serde_json::from_str(json).expect("deserialize");
        assert_eq!(config.script_url, "https://example.com/update.sh");
        assert!(config.github.is_none());
    }

    #[test]
    fn deserialization_with_github() {
        let json = r#"{
            "script_url": "https://example.com/update.sh",
            "github": {
                "owner": "BookLore",
                "repo": "BookLore"
            }
        }"#;
        let config: ProxmoxHelperScriptsConfig = serde_json::from_str(json).expect("deserialize");
        assert_eq!(config.script_url, "https://example.com/update.sh");
        let gh = config.github.as_ref().expect("github should be present");
        assert_eq!(gh.owner, "BookLore");
        assert_eq!(gh.repo, "BookLore");
        assert!(gh.auth_token.is_none());
        assert_eq!(gh.tag_strip_prefix, "v");
        assert!(!gh.include_prereleases);
    }

    #[test]
    fn serialization_roundtrip_without_github() {
        let config = ProxmoxHelperScriptsConfig {
            script_url: "https://example.com/update.sh".to_string(),
            github: None,
        };
        let json = serde_json::to_string(&config).expect("serialize");
        assert!(!json.contains("github"));
        let deserialized: ProxmoxHelperScriptsConfig =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.script_url, config.script_url);
        assert!(deserialized.github.is_none());
    }

    #[test]
    fn serialization_roundtrip_with_github() {
        let config = ProxmoxHelperScriptsConfig {
            script_url: "https://example.com/update.sh".to_string(),
            github: Some(GitHubReleaseSource {
                owner: "owner".to_string(),
                repo: "repo".to_string(),
                auth_token: Some(SecretString::new("ghp_test".to_string())),
                tag_strip_prefix: "v".to_string(),
                include_prereleases: true,
            }),
        };
        let json = serde_json::to_string(&config).expect("serialize");
        let deserialized: ProxmoxHelperScriptsConfig =
            serde_json::from_str(&json).expect("deserialize");
        let gh = deserialized.github.as_ref().expect("github present");
        assert_eq!(gh.owner, "owner");
        assert_eq!(gh.repo, "repo");
        assert_eq!(
            gh.auth_token.as_ref().expect("token").expose_secret(),
            "ghp_test"
        );
        assert!(gh.include_prereleases);
    }

    #[test]
    fn validate_github_empty_owner_fails() {
        let config = ProxmoxHelperScriptsConfig {
            script_url: "https://example.com/update.sh".to_string(),
            github: Some(GitHubReleaseSource {
                owner: String::new(),
                repo: "repo".to_string(),
                auth_token: None,
                tag_strip_prefix: "v".to_string(),
                include_prereleases: false,
            }),
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("owner"));
    }

    #[test]
    fn validate_github_empty_repo_fails() {
        let config = ProxmoxHelperScriptsConfig {
            script_url: "https://example.com/update.sh".to_string(),
            github: Some(GitHubReleaseSource {
                owner: "owner".to_string(),
                repo: String::new(),
                auth_token: None,
                tag_strip_prefix: "v".to_string(),
                include_prereleases: false,
            }),
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("repo"));
    }

    #[test]
    fn validate_github_owner_with_slash_fails() {
        let config = ProxmoxHelperScriptsConfig {
            script_url: "https://example.com/update.sh".to_string(),
            github: Some(GitHubReleaseSource {
                owner: "bad/owner".to_string(),
                repo: "repo".to_string(),
                auth_token: None,
                tag_strip_prefix: "v".to_string(),
                include_prereleases: false,
            }),
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_github_repo_with_traversal_fails() {
        let config = ProxmoxHelperScriptsConfig {
            script_url: "https://example.com/update.sh".to_string(),
            github: Some(GitHubReleaseSource {
                owner: "owner".to_string(),
                repo: "../bad".to_string(),
                auth_token: None,
                tag_strip_prefix: "v".to_string(),
                include_prereleases: false,
            }),
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_github_valid_passes() {
        let config = ProxmoxHelperScriptsConfig {
            script_url: "https://example.com/update.sh".to_string(),
            github: Some(GitHubReleaseSource {
                owner: "BookLore".to_string(),
                repo: "BookLore".to_string(),
                auth_token: None,
                tag_strip_prefix: "v".to_string(),
                include_prereleases: false,
            }),
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn secret_masking_without_github() {
        let config = ProxmoxHelperScriptsConfig {
            script_url: "https://example.com/update.sh".to_string(),
            github: None,
        };
        let masked = config.with_secrets_masked();
        assert!(masked.github.is_none());
    }

    #[test]
    fn secret_masking_with_github_masks_token() {
        let config = ProxmoxHelperScriptsConfig {
            script_url: "https://example.com/update.sh".to_string(),
            github: Some(GitHubReleaseSource {
                owner: "owner".to_string(),
                repo: "repo".to_string(),
                auth_token: Some(SecretString::new("ghp_real".to_string())),
                tag_strip_prefix: "v".to_string(),
                include_prereleases: false,
            }),
        };
        let masked = config.with_secrets_masked();
        let gh = masked.github.as_ref().expect("github present");
        assert_eq!(
            gh.auth_token.as_ref().expect("token").expose_secret(),
            SECRET_MASK
        );
        assert_eq!(gh.owner, "owner");
    }

    #[test]
    fn secret_masking_with_github_no_token_shows_mask() {
        let config = ProxmoxHelperScriptsConfig {
            script_url: "https://example.com/update.sh".to_string(),
            github: Some(GitHubReleaseSource {
                owner: "owner".to_string(),
                repo: "repo".to_string(),
                auth_token: None,
                tag_strip_prefix: "v".to_string(),
                include_prereleases: false,
            }),
        };
        let masked = config.with_secrets_masked();
        let gh = masked.github.as_ref().expect("github present");
        assert_eq!(
            gh.auth_token.as_ref().expect("token").expose_secret(),
            SECRET_MASK
        );
    }

    #[test]
    fn secret_restore_restores_masked_token() {
        let existing = ProxmoxHelperScriptsConfig {
            script_url: "https://example.com/update.sh".to_string(),
            github: Some(GitHubReleaseSource {
                owner: "owner".to_string(),
                repo: "repo".to_string(),
                auth_token: Some(SecretString::new("ghp_real_token".to_string())),
                tag_strip_prefix: "v".to_string(),
                include_prereleases: false,
            }),
        };
        let mut incoming = existing.clone().with_secrets_masked();
        incoming.restore_secrets_from(&existing);
        let gh = incoming.github.as_ref().expect("github present");
        assert_eq!(
            gh.auth_token.as_ref().expect("token").expose_secret(),
            "ghp_real_token"
        );
    }

    #[test]
    fn secret_restore_keeps_new_token() {
        let existing = ProxmoxHelperScriptsConfig {
            script_url: "https://example.com/update.sh".to_string(),
            github: Some(GitHubReleaseSource {
                owner: "owner".to_string(),
                repo: "repo".to_string(),
                auth_token: Some(SecretString::new("ghp_old".to_string())),
                tag_strip_prefix: "v".to_string(),
                include_prereleases: false,
            }),
        };
        let mut incoming = ProxmoxHelperScriptsConfig {
            script_url: "https://example.com/update.sh".to_string(),
            github: Some(GitHubReleaseSource {
                owner: "owner".to_string(),
                repo: "repo".to_string(),
                auth_token: Some(SecretString::new("ghp_new".to_string())),
                tag_strip_prefix: "v".to_string(),
                include_prereleases: false,
            }),
        };
        incoming.restore_secrets_from(&existing);
        let gh = incoming.github.as_ref().expect("github present");
        assert_eq!(
            gh.auth_token.as_ref().expect("token").expose_secret(),
            "ghp_new"
        );
    }

    #[test]
    fn secret_restore_noop_when_no_github() {
        let existing = ProxmoxHelperScriptsConfig {
            script_url: "https://example.com/update.sh".to_string(),
            github: None,
        };
        let mut incoming = ProxmoxHelperScriptsConfig {
            script_url: "https://example.com/update.sh".to_string(),
            github: None,
        };
        incoming.restore_secrets_from(&existing);
        assert!(incoming.github.is_none());
    }
}
