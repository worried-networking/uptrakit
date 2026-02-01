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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_release() {
        let json = serde_json::json!({
            "tag_name": "v1.0.0",
            "name": "Release 1.0.0",
            "draft": false,
            "prerelease": false,
            "html_url": "https://github.com/owner/repo/releases/tag/v1.0.0",
            "body": "## Changes\n- Initial release",
            "published_at": "2024-01-28T00:00:00Z",
            "assets": [
                {
                    "name": "app-linux-amd64.tar.gz",
                    "browser_download_url": "https://github.com/owner/repo/releases/download/v1.0.0/app-linux-amd64.tar.gz",
                    "size": 12345678,
                    "content_type": "application/gzip"
                }
            ]
        });

        let release: GitHubRelease = serde_json::from_value(json).expect("deserialize");
        assert_eq!(release.tag_name, "v1.0.0");
        assert_eq!(release.name.as_deref(), Some("Release 1.0.0"));
        assert!(!release.draft);
        assert!(!release.prerelease);
        assert_eq!(release.assets.len(), 1);
        assert_eq!(release.assets[0].name, "app-linux-amd64.tar.gz");
        assert_eq!(release.assets[0].size, 12345678);
    }

    #[test]
    fn deserialize_release_minimal() {
        let json = serde_json::json!({
            "tag_name": "v0.1.0",
            "name": null,
            "draft": true,
            "prerelease": true,
            "html_url": "https://github.com/owner/repo/releases/tag/v0.1.0",
            "body": null,
            "published_at": null
        });

        let release: GitHubRelease = serde_json::from_value(json).expect("deserialize");
        assert_eq!(release.tag_name, "v0.1.0");
        assert!(release.name.is_none());
        assert!(release.draft);
        assert!(release.prerelease);
        assert!(release.body.is_none());
        assert!(release.published_at.is_none());
        assert!(release.assets.is_empty());
    }

    #[test]
    fn deserialize_api_error() {
        let json = serde_json::json!({
            "message": "API rate limit exceeded for 1.2.3.4.",
            "documentation_url": "https://docs.github.com/rest/overview/resources-in-the-rest-api#rate-limiting"
        });

        let error: GitHubApiError = serde_json::from_value(json).expect("deserialize");
        assert!(error.message.contains("rate limit"));
        assert!(error.documentation_url.is_some());
    }

    #[test]
    fn deserialize_asset() {
        let json = serde_json::json!({
            "name": "checksums.txt",
            "browser_download_url": "https://github.com/o/r/releases/download/v1/checksums.txt",
            "size": 256,
            "content_type": "text/plain"
        });

        let asset: GitHubAsset = serde_json::from_value(json).expect("deserialize");
        assert_eq!(asset.name, "checksums.txt");
        assert_eq!(asset.size, 256);
        assert_eq!(asset.content_type.as_deref(), Some("text/plain"));
    }

    #[test]
    fn deserialize_release_with_extra_fields() {
        // GitHub API returns many fields we don't map — ensure they're ignored
        let json = serde_json::json!({
            "tag_name": "v2.0.0",
            "name": "v2.0.0",
            "draft": false,
            "prerelease": false,
            "html_url": "https://github.com/owner/repo/releases/tag/v2.0.0",
            "body": "Release notes",
            "published_at": "2024-06-01T12:00:00Z",
            "assets": [],
            "id": 12345,
            "node_id": "abcdef",
            "author": {"login": "octocat"},
            "target_commitish": "main",
            "created_at": "2024-06-01T11:00:00Z",
            "tarball_url": "https://api.github.com/repos/owner/repo/tarball/v2.0.0",
            "zipball_url": "https://api.github.com/repos/owner/repo/zipball/v2.0.0"
        });

        let release: GitHubRelease = serde_json::from_value(json).expect("deserialize");
        assert_eq!(release.tag_name, "v2.0.0");
    }
}
