use serde::Deserialize;

/// GitLab API release representation.
#[derive(Debug, Clone, Deserialize)]
pub struct GitLabRelease {
    pub tag_name: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub released_at: Option<String>,
    /// When `true`, the release is not yet publicly visible (upcoming release).
    /// This is GitLab's closest concept to a "pre-release" or "draft" status.
    #[serde(default)]
    pub upcoming_release: bool,
    pub assets: GitLabReleaseAssets,
}

/// Container for GitLab release assets.
#[derive(Debug, Clone, Deserialize)]
pub struct GitLabReleaseAssets {
    /// Manually-uploaded asset links. These are the user-provided downloads,
    /// as opposed to the auto-generated source archives GitLab always provides.
    #[serde(default)]
    pub links: Vec<GitLabReleaseLink>,
}

/// A single asset link in a GitLab release.
#[derive(Debug, Clone, Deserialize)]
pub struct GitLabReleaseLink {
    pub name: String,
    pub url: String,
}

/// GitLab API error response.
#[derive(Debug, Clone, Deserialize)]
pub struct GitLabApiError {
    pub message: String,
}
