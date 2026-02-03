/// Sentinel value used to indicate a masked secret in API responses.
const SECRET_MASK: &str = "***";

/// Mask secret fields in provider config JSON before returning to the client.
pub fn mask_secrets(provider_type: &str, config: &serde_json::Value) -> serde_json::Value {
    let mut masked = config.clone();
    match provider_type {
        "github_releases" => {
            if let Some(obj) = masked.as_object_mut()
                && let Some(token) = obj.get("auth_token")
                && !token.is_null()
            {
                obj.insert(
                    "auth_token".to_string(),
                    serde_json::Value::String(SECRET_MASK.to_string()),
                );
            }
        }
        "docker_registry" => {
            if let Some(obj) = masked.as_object_mut()
                && let Some(auth) = obj.get_mut("auth")
                && let Some(auth_obj) = auth.as_object_mut()
            {
                if let Some(password) = auth_obj.get("password")
                    && !password.is_null()
                {
                    auth_obj.insert(
                        "password".to_string(),
                        serde_json::Value::String(SECRET_MASK.to_string()),
                    );
                }
                if let Some(token) = auth_obj.get("token")
                    && !token.is_null()
                {
                    auth_obj.insert(
                        "token".to_string(),
                        serde_json::Value::String(SECRET_MASK.to_string()),
                    );
                }
            }
        }
        _ => {}
    }
    masked
}

/// Restore preserved secrets from the existing DB value when the client sends the mask sentinel.
pub fn restore_secrets(
    provider_type: &str,
    incoming: &mut serde_json::Value,
    existing: &serde_json::Value,
) {
    match provider_type {
        "github_releases" => {
            if let (Some(incoming_obj), Some(existing_obj)) =
                (incoming.as_object_mut(), existing.as_object())
                && let Some(token) = incoming_obj.get("auth_token")
                && token.as_str() == Some(SECRET_MASK)
                && let Some(existing_token) = existing_obj.get("auth_token")
            {
                incoming_obj.insert("auth_token".to_string(), existing_token.clone());
            }
        }
        "docker_registry" => {
            if let (Some(incoming_obj), Some(existing_obj)) =
                (incoming.as_object_mut(), existing.as_object())
                && let Some(incoming_auth) = incoming_obj.get_mut("auth")
                && let Some(incoming_auth_obj) = incoming_auth.as_object_mut()
                && let Some(existing_auth) = existing_obj.get("auth")
                && let Some(existing_auth_obj) = existing_auth.as_object()
            {
                if let Some(password) = incoming_auth_obj.get("password")
                    && password.as_str() == Some(SECRET_MASK)
                    && let Some(existing_password) = existing_auth_obj.get("password")
                {
                    incoming_auth_obj.insert("password".to_string(), existing_password.clone());
                }
                if let Some(token) = incoming_auth_obj.get("token")
                    && token.as_str() == Some(SECRET_MASK)
                    && let Some(existing_token) = existing_auth_obj.get("token")
                {
                    incoming_auth_obj.insert("token".to_string(), existing_token.clone());
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_github_auth_token() {
        let config = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world",
            "auth_token": "ghp_secret123"
        });
        let masked = mask_secrets("github_releases", &config);
        assert_eq!(masked["auth_token"], SECRET_MASK);
        assert_eq!(masked["owner"], "octocat");
    }

    #[test]
    fn mask_preserves_null_token() {
        let config = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world",
            "auth_token": null
        });
        let masked = mask_secrets("github_releases", &config);
        assert!(masked["auth_token"].is_null());
    }

    #[test]
    fn mask_without_token_field() {
        let config = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world"
        });
        let masked = mask_secrets("github_releases", &config);
        // No auth_token field should be added
        assert!(masked.get("auth_token").is_none());
    }

    #[test]
    fn mask_unknown_provider_type() {
        let config = serde_json::json!({"key": "value"});
        let masked = mask_secrets("unknown_type", &config);
        assert_eq!(masked, config);
    }

    #[test]
    fn restore_masked_token() {
        let mut incoming = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world",
            "auth_token": "***"
        });
        let existing = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world",
            "auth_token": "ghp_real_token"
        });
        restore_secrets("github_releases", &mut incoming, &existing);
        assert_eq!(incoming["auth_token"], "ghp_real_token");
    }

    #[test]
    fn restore_new_token_not_masked() {
        let mut incoming = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world",
            "auth_token": "ghp_new_token"
        });
        let existing = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world",
            "auth_token": "ghp_old_token"
        });
        restore_secrets("github_releases", &mut incoming, &existing);
        assert_eq!(incoming["auth_token"], "ghp_new_token");
    }

    #[test]
    fn mask_docker_registry_basic_password() {
        let config = serde_json::json!({
            "image": "nginx",
            "auth": {
                "type": "basic",
                "username": "user",
                "password": "secret123"
            }
        });
        let masked = mask_secrets("docker_registry", &config);
        assert_eq!(masked["auth"]["password"], SECRET_MASK);
        assert_eq!(masked["auth"]["username"], "user");
    }

    #[test]
    fn mask_docker_registry_bearer_token() {
        let config = serde_json::json!({
            "image": "nginx",
            "auth": {
                "type": "bearer",
                "token": "ghcr_token_secret"
            }
        });
        let masked = mask_secrets("docker_registry", &config);
        assert_eq!(masked["auth"]["token"], SECRET_MASK);
    }

    #[test]
    fn mask_docker_registry_no_auth() {
        let config = serde_json::json!({
            "image": "nginx"
        });
        let masked = mask_secrets("docker_registry", &config);
        assert!(masked.get("auth").is_none());
    }

    #[test]
    fn restore_docker_registry_masked_password() {
        let mut incoming = serde_json::json!({
            "image": "nginx",
            "auth": {
                "type": "basic",
                "username": "user",
                "password": "***"
            }
        });
        let existing = serde_json::json!({
            "image": "nginx",
            "auth": {
                "type": "basic",
                "username": "user",
                "password": "real_password"
            }
        });
        restore_secrets("docker_registry", &mut incoming, &existing);
        assert_eq!(incoming["auth"]["password"], "real_password");
    }

    #[test]
    fn restore_docker_registry_masked_token() {
        let mut incoming = serde_json::json!({
            "image": "nginx",
            "auth": {
                "type": "bearer",
                "token": "***"
            }
        });
        let existing = serde_json::json!({
            "image": "nginx",
            "auth": {
                "type": "bearer",
                "token": "real_token"
            }
        });
        restore_secrets("docker_registry", &mut incoming, &existing);
        assert_eq!(incoming["auth"]["token"], "real_token");
    }
}
