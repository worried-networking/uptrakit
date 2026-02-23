//! URL path constants and constructors for all Uptrakit API endpoints.
//!
//! This is the single source of truth for every path used by the typed client
//! methods and the mock server helpers. When an API path changes, update it
//! here and the compiler will catch every stale call site.
//!
//! # Conventions
//!
//! - `pub(crate) const NAME: &str` — static paths with no runtime parameters.
//! - `pub(crate) fn name(id: &Uuid) -> String` — paths that embed one or more
//!   IDs resolved at call time.

pub(crate) mod auth {
    /// `POST /api/v1/auth/register`
    pub(crate) const REGISTER: &str = "/api/v1/auth/register";
    /// `POST /api/v1/auth/login`
    pub(crate) const LOGIN: &str = "/api/v1/auth/login";
    /// `POST /api/v1/auth/refresh`
    pub(crate) const REFRESH: &str = "/api/v1/auth/refresh";
    /// `POST /api/v1/auth/logout`
    pub(crate) const LOGOUT: &str = "/api/v1/auth/logout";
    /// `GET /api/v1/auth/me`
    pub(crate) const ME: &str = "/api/v1/auth/me";
    /// `GET /api/v1/auth/methods`
    pub(crate) const METHODS: &str = "/api/v1/auth/methods";
    /// `POST /api/v1/auth/device`
    pub(crate) const DEVICE: &str = "/api/v1/auth/device";
    /// `POST /api/v1/auth/device/poll`
    pub(crate) const DEVICE_POLL: &str = "/api/v1/auth/device/poll";
    /// `POST /api/v1/auth/device/approve`
    pub(crate) const DEVICE_APPROVE: &str = "/api/v1/auth/device/approve";
}

pub(crate) mod api_tokens {
    use uuid::Uuid;
    /// `GET /api/v1/auth/api-tokens` · `POST /api/v1/auth/api-tokens`
    pub(crate) const BASE: &str = "/api/v1/auth/api-tokens";
    /// `DELETE /api/v1/auth/api-tokens/{id}`
    pub(crate) fn by_id(id: &Uuid) -> String {
        format!("/api/v1/auth/api-tokens/{id}")
    }
}

pub(crate) mod health {
    /// `GET /healthz`
    pub(crate) const HEALTHZ: &str = "/healthz";
}

pub(crate) mod autodiscovery {
    use uuid::Uuid;
    /// `GET /api/v1/autodiscovery/ignores` · `POST …`
    pub(crate) const IGNORES: &str = "/api/v1/autodiscovery/ignores";
    /// `DELETE /api/v1/autodiscovery/ignores/{id}`
    pub(crate) fn ignore_by_id(id: &Uuid) -> String {
        format!("/api/v1/autodiscovery/ignores/{id}")
    }
}

pub(crate) mod hosts {
    use uuid::Uuid;
    /// `GET /api/v1/hosts`
    pub(crate) const BASE: &str = "/api/v1/hosts";
    /// `GET /api/v1/hosts/{id}` · `PUT /api/v1/hosts/{id}` · `DELETE /api/v1/hosts/{id}`
    pub(crate) fn by_id(id: &Uuid) -> String {
        format!("/api/v1/hosts/{id}")
    }
    /// `POST /api/v1/hosts/{id}/discover`
    pub(crate) fn discover(id: &Uuid) -> String {
        format!("/api/v1/hosts/{id}/discover")
    }
    /// `DELETE /api/v1/hosts/{id}/discovered`
    pub(crate) fn discovered(id: &Uuid) -> String {
        format!("/api/v1/hosts/{id}/discovered")
    }
}

pub(crate) mod oidc_auth {
    use uuid::Uuid;
    /// `GET /api/v1/auth/oidc/{provider_id}/authorize`
    pub(crate) fn authorize(provider_id: &Uuid) -> String {
        format!("/api/v1/auth/oidc/{provider_id}/authorize")
    }
    /// `POST /api/v1/auth/oidc/exchange`
    pub(crate) const EXCHANGE: &str = "/api/v1/auth/oidc/exchange";
    /// `POST /api/v1/auth/oidc/link`
    pub(crate) const LINK: &str = "/api/v1/auth/oidc/link";
    /// `POST /api/v1/auth/oidc/complete-registration`
    pub(crate) const COMPLETE_REGISTRATION: &str = "/api/v1/auth/oidc/complete-registration";
}

pub(crate) mod oidc_providers {
    use uuid::Uuid;
    /// `GET /api/v1/settings/oidc-providers` · `POST /api/v1/settings/oidc-providers`
    pub(crate) const BASE: &str = "/api/v1/settings/oidc-providers";
    /// `GET /api/v1/settings/oidc-providers/{id}` · `PUT …` · `DELETE …`
    pub(crate) fn by_id(id: &Uuid) -> String {
        format!("/api/v1/settings/oidc-providers/{id}")
    }
    /// `POST /api/v1/settings/oidc-providers/{id}/activate`
    pub(crate) fn activate(id: &Uuid) -> String {
        format!("/api/v1/settings/oidc-providers/{id}/activate")
    }
    /// `POST /api/v1/settings/oidc-providers/{id}/deactivate`
    pub(crate) fn deactivate(id: &Uuid) -> String {
        format!("/api/v1/settings/oidc-providers/{id}/deactivate")
    }
}

pub(crate) mod pki {
    /// `GET /api/v1/pki/ca.crt`
    pub(crate) const CA_CERT: &str = "/api/v1/pki/ca.crt";
    /// `GET /api/v1/pki/ca.crl`
    pub(crate) const CA_CRL: &str = "/api/v1/pki/ca.crl";
}

pub(crate) mod provider_configs {
    use uuid::Uuid;
    /// `GET /api/v1/provider-configs` · `POST /api/v1/provider-configs`
    pub(crate) const BASE: &str = "/api/v1/provider-configs";
    /// `GET /api/v1/provider-configs/{id}` · `PUT …` · `DELETE …`
    pub(crate) fn by_id(id: &Uuid) -> String {
        format!("/api/v1/provider-configs/{id}")
    }
    /// `POST /api/v1/provider-configs/{id}/discover`
    pub(crate) fn discover(id: &Uuid) -> String {
        format!("/api/v1/provider-configs/{id}/discover")
    }
    /// `DELETE /api/v1/provider-configs/{id}/discovered`
    pub(crate) fn discovered(id: &Uuid) -> String {
        format!("/api/v1/provider-configs/{id}/discovered")
    }
}

pub(crate) mod scheduler {
    use uuid::Uuid;
    /// `GET /api/v1/scheduler/tasks`
    pub(crate) const BASE: &str = "/api/v1/scheduler/tasks";
    /// `GET /api/v1/scheduler/tasks/{id}` · `PUT …`
    pub(crate) fn by_id(id: &Uuid) -> String {
        format!("/api/v1/scheduler/tasks/{id}")
    }
    /// `POST /api/v1/scheduler/tasks/{id}/trigger`
    pub(crate) fn trigger(id: &Uuid) -> String {
        format!("/api/v1/scheduler/tasks/{id}/trigger")
    }
}

pub(crate) mod services {
    use uuid::Uuid;
    /// `GET /api/v1/services` · `POST /api/v1/services`
    pub(crate) const BASE: &str = "/api/v1/services";
    /// `POST /api/v1/services/enrollment-token` · `DELETE …`
    pub(crate) const ENROLLMENT_TOKEN: &str = "/api/v1/services/enrollment-token";
    /// `GET /api/v1/services/enrollment-token/status`
    pub(crate) const ENROLLMENT_TOKEN_STATUS: &str = "/api/v1/services/enrollment-token/status";
    /// `GET /api/v1/services/{id}` · `PUT …` · `DELETE …`
    pub(crate) fn by_id(id: &Uuid) -> String {
        format!("/api/v1/services/{id}")
    }
    /// `POST /api/v1/services/{id}/approve`
    pub(crate) fn approve(id: &Uuid) -> String {
        format!("/api/v1/services/{id}/approve")
    }
    /// `POST /api/v1/services/{id}/reject`
    pub(crate) fn reject(id: &Uuid) -> String {
        format!("/api/v1/services/{id}/reject")
    }
    /// `POST /api/v1/services/{id}/merge`
    pub(crate) fn merge(id: &Uuid) -> String {
        format!("/api/v1/services/{id}/merge")
    }
}

pub(crate) mod settings {
    /// `GET /api/v1/settings`
    pub(crate) const COMBINED: &str = "/api/v1/settings";
    /// `GET /api/v1/settings/registration` · `PUT …`
    pub(crate) const REGISTRATION: &str = "/api/v1/settings/registration";
    /// `GET /api/v1/settings/authentication` · `PUT …`
    pub(crate) const AUTHENTICATION: &str = "/api/v1/settings/authentication";
    /// `GET /api/v1/settings/agent-certificates` · `PUT …`
    pub(crate) const AGENT_CERTIFICATES: &str = "/api/v1/settings/agent-certificates";
    /// `GET /api/v1/settings/network` · `PUT …`
    pub(crate) const NETWORK: &str = "/api/v1/settings/network";
    /// `POST /api/v1/settings/rotate-ca`
    pub(crate) const ROTATE_CA: &str = "/api/v1/settings/rotate-ca";
    /// `POST /api/v1/settings/renew-server-certificate`
    pub(crate) const RENEW_SERVER_CERT: &str = "/api/v1/settings/renew-server-certificate";
}

pub(crate) mod settings_mqtt {
    use uuid::Uuid;
    /// `GET /api/v1/settings/mqtt` · `POST …`
    pub(crate) const BASE: &str = "/api/v1/settings/mqtt";
    /// `GET /api/v1/settings/mqtt/limit` · `PUT …`
    pub(crate) const LIMIT: &str = "/api/v1/settings/mqtt/limit";
    /// `GET /api/v1/settings/mqtt/{id}` · `PUT …` · `DELETE …`
    pub(crate) fn by_id(id: &Uuid) -> String {
        format!("/api/v1/settings/mqtt/{id}")
    }
}

pub(crate) mod software_items {
    use uuid::Uuid;
    /// `GET /api/v1/software-items` · `POST …`
    pub(crate) const BASE: &str = "/api/v1/software-items";
    /// `GET /api/v1/software-items/{id}` · `PUT …` · `DELETE …`
    pub(crate) fn by_id(id: &Uuid) -> String {
        format!("/api/v1/software-items/{id}")
    }
    /// `POST /api/v1/software-items/{id}/hosts`
    pub(crate) fn hosts(id: &Uuid) -> String {
        format!("/api/v1/software-items/{id}/hosts")
    }
    /// `DELETE /api/v1/software-items/{item_id}/hosts/{host_id}`
    pub(crate) fn host(item_id: &Uuid, host_id: &Uuid) -> String {
        format!("/api/v1/software-items/{item_id}/hosts/{host_id}")
    }
    /// `POST /api/v1/software-items/{id}/check-versions`
    pub(crate) fn check_versions(id: &Uuid) -> String {
        format!("/api/v1/software-items/{id}/check-versions")
    }
    /// `POST /api/v1/software-items/{item_id}/hosts/{host_id}/check-versions`
    pub(crate) fn host_check_versions(item_id: &Uuid, host_id: &Uuid) -> String {
        format!("/api/v1/software-items/{item_id}/hosts/{host_id}/check-versions")
    }
    /// `POST /api/v1/software-items/{item_id}/hosts/{host_id}/update`
    pub(crate) fn host_update(item_id: &Uuid, host_id: &Uuid) -> String {
        format!("/api/v1/software-items/{item_id}/hosts/{host_id}/update")
    }
    /// `POST /api/v1/software-items/{id}/approve`
    pub(crate) fn approve(id: &Uuid) -> String {
        format!("/api/v1/software-items/{id}/approve")
    }
}

pub(crate) mod system_alerts {
    /// `GET /api/v1/system/alerts`
    pub(crate) const ALERTS: &str = "/api/v1/system/alerts";
}

pub(crate) mod update_history {
    use uuid::Uuid;
    /// `GET /api/v1/update-history`
    pub(crate) const BASE: &str = "/api/v1/update-history";
    /// `GET /api/v1/update-history/{id}`
    pub(crate) fn by_id(id: &Uuid) -> String {
        format!("/api/v1/update-history/{id}")
    }
}
