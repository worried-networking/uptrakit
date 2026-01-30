/// Business-layer enum combining the `auth_method` string column and
/// `oidc_provider_id` UUID column on the sessions table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthMethod {
    Password,
    Oidc { provider_id: uuid::Uuid },
}

impl AuthMethod {
    /// Construct from the two DB columns.
    pub fn from_session(kind: &str, oidc_provider_id: Option<uuid::Uuid>) -> Option<Self> {
        match kind {
            "password" => Some(Self::Password),
            "oidc" => oidc_provider_id.map(|id| Self::Oidc { provider_id: id }),
            _ => None,
        }
    }

    /// The string value stored in the `auth_method` DB column.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::Oidc { .. } => "oidc",
        }
    }

    /// The provider ID for the `oidc_provider_id` DB column. `None` for password.
    pub fn oidc_provider_id(&self) -> Option<uuid::Uuid> {
        match self {
            Self::Oidc { provider_id } => Some(*provider_id),
            _ => None,
        }
    }
}
