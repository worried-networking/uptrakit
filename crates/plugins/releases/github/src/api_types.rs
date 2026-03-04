use serde::Deserialize;

/// GitHub API release representation.
#[derive(Debug, Clone, Deserialize)]
pub struct GitHubRelease {
    pub tag_name: String,
    pub name: Option<String>,
    pub draft: bool,
    pub prerelease: bool,
    pub html_url: String,
    pub body: Option<String>,
    pub published_at: Option<String>,
    #[serde(default)]
    pub assets: Vec<GitHubAsset>,
}

/// GitHub API asset representation.
#[derive(Debug, Clone, Deserialize)]
pub struct GitHubAsset {
    pub name: String,
    pub browser_download_url: String,
    pub size: u64,
    pub content_type: Option<String>,
}

/// GitHub API error response.
#[derive(Debug, Clone, Deserialize)]
pub struct GitHubApiError {
    pub message: String,
    pub documentation_url: Option<String>,
}

/// GitHub Attestations API response.
///
/// Returned by `GET /repos/{owner}/{repo}/attestations/sha256:{digest}`.
/// An empty `attestations` array means no attestation was found.
/// Individual bundle objects are not inspected — only the array length matters.
#[derive(Debug, Deserialize)]
pub(crate) struct AttestationsApiResponse {
    #[serde(default)]
    pub attestations: Vec<serde_json::Value>,
}
