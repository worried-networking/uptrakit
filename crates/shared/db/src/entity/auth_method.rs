/// Business-layer enum combining the `auth_method` string column and
/// `oidc_provider_id` UUID column on the sessions table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthMethod {
    Password,
    Oidc { provider_id: uuid::Uuid },
    ApiToken,
}

impl AuthMethod {
    /// Construct from the two DB columns.
    pub fn from_session(kind: &str, oidc_provider_id: Option<uuid::Uuid>) -> Option<Self> {
        match kind {
            "password" => Some(Self::Password),
            "oidc" => oidc_provider_id.map(|id| Self::Oidc { provider_id: id }),
            "api_token" => Some(Self::ApiToken),
            _ => None,
        }
    }

    /// The string value stored in the `auth_method` DB column.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::Oidc { .. } => "oidc",
            Self::ApiToken => "api_token",
        }
    }

    /// The provider ID for the `oidc_provider_id` DB column. `None` for password and api_token.
    pub fn oidc_provider_id(&self) -> Option<uuid::Uuid> {
        match self {
            Self::Oidc { provider_id } => Some(*provider_id),
            _ => None,
        }
    }
}
