use serde::Deserialize;

/// Forgejo API release representation.
///
/// The Forgejo/Gitea API mirrors GitHub conventions closely, so the field names
/// are nearly identical.
#[derive(Debug, Clone, Deserialize)]
pub struct ForgejoRelease {
    pub tag_name: String,
    pub name: Option<String>,
    pub draft: bool,
    pub prerelease: bool,
    pub html_url: String,
    pub body: Option<String>,
    pub published_at: Option<String>,
    #[serde(default)]
    pub assets: Vec<ForgejoAsset>,
}

/// Forgejo API release asset representation.
#[derive(Debug, Clone, Deserialize)]
pub struct ForgejoAsset {
    pub name: String,
    pub browser_download_url: String,
    pub size: u64,
}

/// Forgejo API error response.
#[derive(Debug, Clone, Deserialize)]
pub struct ForgejoApiError {
    pub message: String,
}
