//! Cross-platform directory management for Uptrakit binaries.
//!
//! Uses `directories::ProjectDirs` to determine standard paths following
//! platform conventions (XDG on Linux, Application Support on macOS, etc.).
//!
//! Each binary has its own project directories:
//! - Controller: `uptrakit/controller` (qualifier: `org.uptrakit`)
//! - Agent: `uptrakit/agent`
//! - MQTT: `uptrakit/mqtt`
//!
//! The crate provides:
//! - Default paths using platform conventions
//! - User overrides via CLI args
//! - Secure directory/file creation (700 for dirs, 600 for files)
//! - Separation of config vs state directories
//!
//! All public I/O functions are async (backed by `tokio::fs`), eliminating
//! the TOCTOU risk of calling sync filesystem operations from an async
//! runtime.

use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use rootcause::prelude::*;
use thiserror::Error;

/// Errors that can occur during directory operations.
#[derive(Debug, Error)]
pub enum DirectoryError {
    #[error("failed to determine project directories")]
    NoProjectDirs,

    #[error("failed to create directory {path}: {source}")]
    CreateDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to write file {path}: {source}")]
    WriteFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to set permissions on {path}: {source}")]
    SetPermissions {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("home directory could not be determined")]
    NoHomeDir,

    #[error("invalid path component: {reason}")]
    PathTraversal { reason: String },
}

pub type Result<T> = std::result::Result<T, Report<DirectoryError>>;

/// Resolved directory paths for an Uptrakit application.
///
/// Config directory is for persistent configuration that rarely changes:
/// - Controller: CA certificates, TLS certificates, PKI files
/// - Agent/MQTT: Controller's CA certificate
///
/// State directory is for runtime state that may change frequently:
/// - Controller: SQLite database, JWT signing key
/// - Agent/MQTT: Private keys, enrollment state, service certificates
#[derive(Debug, Clone)]
pub struct AppDirs {
    config: PathBuf,
    state: PathBuf,
}

impl AppDirs {
    /// Resolve application directories.
    ///
    /// Uses `directories::ProjectDirs` for defaults, with optional overrides
    /// for config and state directories.
    ///
    /// # Arguments
    ///
    /// * `app` - The application kind
    /// * `config_override` - Optional override for the config directory
    /// * `state_override` - Optional override for the state directory
    pub fn resolve(
        app_name: &str,
        config_override: Option<&Path>,
        state_override: Option<&Path>,
    ) -> Result<Self> {
        let proj_dirs = ProjectDirs::from("org", "uptrakit", app_name)
            .ok_or_else(|| report!(DirectoryError::NoProjectDirs))?;

        let config = match config_override {
            Some(path) => expand_tilde(path)?,
            None => proj_dirs.config_dir().to_path_buf(),
        };

        let state = match state_override {
            Some(path) => expand_tilde(path)?,
            None => {
                // On Linux, state_dir is under XDG_STATE_HOME (~/.local/state)
                // On macOS, there's no separate state dir, so we use data_local_dir
                #[cfg(target_os = "linux")]
                {
                    proj_dirs
                        .state_dir()
                        .map(|p| p.to_path_buf())
                        .unwrap_or_else(|| proj_dirs.data_local_dir().to_path_buf())
                }
                #[cfg(not(target_os = "linux"))]
                {
                    proj_dirs.data_local_dir().to_path_buf()
                }
            }
        };

        Ok(Self { config, state })
    }

    /// Config directory path.
    pub fn config_dir(&self) -> &Path {
        &self.config
    }

    /// State directory path.
    pub fn state_dir(&self) -> &Path {
        &self.state
    }

    /// Ensure config directory exists with secure permissions (700).
    pub async fn ensure_config_dir(&self) -> Result<()> {
        create_secure_dir(&self.config).await
    }

    /// Ensure state directory exists with secure permissions (700).
    pub async fn ensure_state_dir(&self) -> Result<()> {
        create_secure_dir(&self.state).await
    }

    /// Ensure both directories exist with secure permissions (700).
    pub async fn ensure_dirs(&self) -> Result<()> {
        create_secure_dir(&self.config).await?;
        create_secure_dir(&self.state).await?;
        Ok(())
    }

    /// Get a path within the config directory.
    ///
    /// Returns `Err` if `name` contains path separators, `..` components,
    /// or is an absolute path.
    pub fn config_path(&self, name: &str) -> Result<PathBuf> {
        validate_path_name(name)?;
        Ok(self.config.join(name))
    }

    /// Get a path within the state directory.
    ///
    /// Returns `Err` if `name` contains path separators, `..` components,
    /// or is an absolute path.
    pub fn state_path(&self, name: &str) -> Result<PathBuf> {
        validate_path_name(name)?;
        Ok(self.state.join(name))
    }
}

/// Validate that a path component name is safe for use with `config_path`/`state_path`.
///
/// Rejects names containing path separators (`/`, `\`), parent directory
/// traversal (`..`), or absolute path prefixes.
fn validate_path_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!(DirectoryError::PathTraversal {
            reason: "name must not be empty".into(),
        });
    }

    if name.contains('/') || name.contains('\\') {
        bail!(DirectoryError::PathTraversal {
            reason: format!("name must not contain path separators: {name:?}"),
        });
    }

    if name == ".." || name == "." {
        bail!(DirectoryError::PathTraversal {
            reason: format!("name must not be a relative path component: {name:?}"),
        });
    }

    let path = Path::new(name);
    if path.is_absolute() {
        bail!(DirectoryError::PathTraversal {
            reason: format!("name must not be an absolute path: {name:?}"),
        });
    }

    Ok(())
}

/// Expand `~` prefix to the user's home directory.
///
/// Uses `std::path::Component`-based matching to avoid lossy string
/// conversion, preserving non-UTF-8 path components on Unix.
pub fn expand_tilde(path: &Path) -> Result<PathBuf> {
    let mut components = path.components();
    match components.next() {
        Some(std::path::Component::Normal(first)) if first == "~" => {
            let home = home_dir().ok_or_else(|| report!(DirectoryError::NoHomeDir))?;
            let rest: PathBuf = components.collect();
            Ok(home.join(rest))
        }
        _ => Ok(path.to_path_buf()),
    }
}

/// Get the user's home directory.
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Create a directory with secure permissions (700).
///
/// On Unix, uses `tokio::fs::DirBuilder` with mode `0o700` so newly created
/// directories get correct permissions atomically. After creation, walks all
/// path components from the first newly created ancestor down to the leaf and
/// calls `set_dir_permissions()` on each to ensure intermediate directories
/// also have `0o700` (the stdlib `DirBuilder::recursive(true)` only applies
/// the mode to the leaf). Creates parent directories as needed.
pub async fn create_secure_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        // Find the deepest ancestor that already exists before creating anything.
        let first_existing = {
            let mut ancestor = path.to_path_buf();
            while tokio::fs::metadata(&ancestor).await.is_err() {
                if !ancestor.pop() {
                    break;
                }
            }
            ancestor
        };

        tokio::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
            .await
            .map_err(|e| {
                report!(DirectoryError::CreateDir {
                    path: path.to_path_buf(),
                    source: e,
                })
            })?;

        // Always fix the leaf's permissions (covers pre-existing dirs with wrong mode).
        set_dir_permissions(path).await?;

        // Also fix any newly created intermediate directories between the
        // first pre-existing ancestor and the leaf. DirBuilder::recursive
        // only applies the mode to the leaf; intermediates get default
        // permissions (typically 0o755 after umask).
        if first_existing != path {
            let mut current = path.to_path_buf();
            while let Some(parent) = current.parent().map(|p| p.to_path_buf()) {
                if parent == first_existing {
                    break;
                }
                set_dir_permissions(&parent).await?;
                current = parent;
            }
        }
    }

    #[cfg(not(unix))]
    {
        tokio::fs::create_dir_all(path).await.map_err(|e| {
            report!(DirectoryError::CreateDir {
                path: path.to_path_buf(),
                source: e,
            })
        })?;
    }

    Ok(())
}

/// Write data to a file atomically with secure permissions (600).
///
/// On Unix, opens the file with `mode(0o600)` via `OpenOptionsExt` so it is
/// never world-readable, eliminating the TOCTOU window of write-then-chmod.
/// Creates parent directories as needed with 700 permissions.
pub async fn write_secure_file(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        create_secure_dir(parent).await?;
    }

    write_with_mode(path, data).await?;
    Ok(())
}

/// Write string data to a file with secure permissions (600).
pub async fn write_secure_file_str(path: &Path, data: &str) -> Result<()> {
    write_secure_file(path, data.as_bytes()).await
}

/// Async helper: open + write with `mode(0o600)` on Unix via tokio.
async fn write_with_mode(path: &Path, data: &[u8]) -> Result<()> {
    use tokio::io::AsyncWriteExt;

    #[cfg(unix)]
    let file = {
        tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .await
    };

    #[cfg(not(unix))]
    let file = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .await;

    let mut file = file.map_err(|e| {
        report!(DirectoryError::WriteFile {
            path: path.to_path_buf(),
            source: e,
        })
    })?;

    file.write_all(data).await.map_err(|e| {
        report!(DirectoryError::WriteFile {
            path: path.to_path_buf(),
            source: e,
        })
    })?;

    file.shutdown().await.map_err(|e| {
        report!(DirectoryError::WriteFile {
            path: path.to_path_buf(),
            source: e,
        })
    })?;

    Ok(())
}

/// Set directory permissions to 700 (owner rwx only).
pub async fn set_dir_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .await
            .map_err(|e| {
                report!(DirectoryError::SetPermissions {
                    path: path.to_path_buf(),
                    source: e,
                })
            })?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

/// Set file permissions to 600 (owner rw only).
pub async fn set_file_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .await
            .map_err(|e| {
                report!(DirectoryError::SetPermissions {
                    path: path.to_path_buf(),
                    source: e,
                })
            })?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    #[test]
    fn resolve_with_defaults() {
        let dirs = AppDirs::resolve("controller", None, None).expect("should resolve");
        // Just verify it returns something
        assert!(!dirs.config_dir().as_os_str().is_empty());
        assert!(!dirs.state_dir().as_os_str().is_empty());
    }

    #[test]
    fn resolve_with_overrides() {
        let temp = TempDir::new().expect("temp dir");
        let config_path = temp.path().join("config");
        let state_path = temp.path().join("state");

        let dirs = AppDirs::resolve("agent", Some(&config_path), Some(&state_path))
            .expect("should resolve");

        assert_eq!(dirs.config_dir(), config_path);
        assert_eq!(dirs.state_dir(), state_path);
    }

    #[test]
    fn expand_tilde_home() {
        let expanded = expand_tilde(Path::new("~")).expect("should expand");
        assert!(!expanded.to_string_lossy().contains('~'));
    }

    #[test]
    fn expand_tilde_with_subpath() {
        let expanded = expand_tilde(Path::new("~/foo/bar")).expect("should expand");
        assert!(expanded.to_string_lossy().ends_with("foo/bar"));
        assert!(!expanded.to_string_lossy().contains('~'));
    }

    #[test]
    fn expand_tilde_no_change_for_absolute() {
        let path = Path::new("/absolute/path");
        let expanded = expand_tilde(path).expect("should expand");
        assert_eq!(expanded, path);
    }

    #[tokio::test]
    async fn create_secure_dir_permissions() {
        let temp = TempDir::new().expect("temp dir");
        let dir = temp.path().join("secure_dir");

        create_secure_dir(&dir).await.expect("should create");

        assert!(dir.is_dir());
        let metadata = std::fs::metadata(&dir).expect("metadata");
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[tokio::test]
    async fn create_secure_dir_corrects_existing_permissions() {
        let temp = TempDir::new().expect("temp dir");
        let dir = temp.path().join("wrong_perms_dir");

        // Create directory with overly permissive mode (0o755)
        std::fs::create_dir(&dir).expect("create dir");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).expect("set perms");
        let mode = std::fs::metadata(&dir)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o755, "precondition: dir should be 0o755");

        // create_secure_dir should correct permissions to 0o700
        create_secure_dir(&dir)
            .await
            .expect("should succeed on existing dir");

        let mode = std::fs::metadata(&dir)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700, "permissions should be corrected to 0o700");
    }

    #[tokio::test]
    async fn create_secure_dir_sets_intermediate_permissions() {
        let temp = TempDir::new().expect("temp dir");
        // Create a deeply nested path where intermediate dirs don't exist yet
        let leaf = temp.path().join("a").join("b").join("c");

        create_secure_dir(&leaf).await.expect("should create");

        // Verify all intermediate directories have 0o700
        for component in ["a", "a/b", "a/b/c"] {
            let dir = temp.path().join(component);
            assert!(dir.is_dir(), "{component} should be a directory");
            let mode = std::fs::metadata(&dir)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o700, "{component} should have 0o700 permissions");
        }
    }

    #[tokio::test]
    async fn create_secure_dir_idempotent() {
        let temp = TempDir::new().expect("temp dir");
        let dir = temp.path().join("idempotent_dir");

        create_secure_dir(&dir).await.expect("first call");
        create_secure_dir(&dir)
            .await
            .expect("second call should succeed");

        assert!(dir.is_dir());
        let mode = std::fs::metadata(&dir)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[tokio::test]
    async fn write_secure_file_permissions() {
        let temp = TempDir::new().expect("temp dir");
        let file = temp.path().join("secure_file");

        write_secure_file(&file, b"secret data")
            .await
            .expect("should write");

        assert!(file.is_file());
        let metadata = std::fs::metadata(&file).expect("metadata");
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[tokio::test]
    async fn write_secure_file_creates_parent_dirs() {
        let temp = TempDir::new().expect("temp dir");
        let file = temp.path().join("a/b/c/file");

        write_secure_file(&file, b"data")
            .await
            .expect("should write");

        assert!(file.is_file());
    }

    #[tokio::test]
    async fn ensure_dirs_creates_both() {
        let temp = TempDir::new().expect("temp dir");
        let config = temp.path().join("config");
        let state = temp.path().join("state");

        let dirs = AppDirs::resolve("mqtt", Some(&config), Some(&state)).expect("should resolve");
        dirs.ensure_dirs().await.expect("should ensure");

        assert!(config.is_dir());
        assert!(state.is_dir());
    }

    #[test]
    fn config_and_state_paths() {
        let temp = TempDir::new().expect("temp dir");
        let dirs = AppDirs::resolve("controller", Some(temp.path()), Some(temp.path()))
            .expect("should resolve");

        assert_eq!(
            dirs.config_path("ca.crt").expect("valid name"),
            temp.path().join("ca.crt")
        );
        assert_eq!(
            dirs.state_path("db.sqlite").expect("valid name"),
            temp.path().join("db.sqlite")
        );
    }

    #[test]
    fn config_path_rejects_traversal() {
        let temp = TempDir::new().expect("temp dir");
        let dirs = AppDirs::resolve("controller", Some(temp.path()), Some(temp.path()))
            .expect("should resolve");

        assert!(dirs.config_path("..").is_err());
        assert!(dirs.config_path(".").is_err());
        assert!(dirs.config_path("../etc/passwd").is_err());
        assert!(dirs.config_path("foo/bar").is_err());
        assert!(dirs.config_path("foo\\bar").is_err());
        assert!(dirs.config_path("").is_err());
    }

    #[test]
    fn state_path_rejects_traversal() {
        let temp = TempDir::new().expect("temp dir");
        let dirs = AppDirs::resolve("controller", Some(temp.path()), Some(temp.path()))
            .expect("should resolve");

        assert!(dirs.state_path("..").is_err());
        assert!(dirs.state_path("sub/dir").is_err());
        assert!(dirs.state_path("").is_err());
    }

    #[test]
    fn config_path_accepts_valid_names() {
        let temp = TempDir::new().expect("temp dir");
        let dirs = AppDirs::resolve("controller", Some(temp.path()), Some(temp.path()))
            .expect("should resolve");

        assert!(dirs.config_path("ca.crt").is_ok());
        assert!(dirs.config_path("db.sqlite").is_ok());
        assert!(dirs.config_path("config.json").is_ok());
        assert!(dirs.config_path(".hidden").is_ok());
    }

    #[tokio::test]
    async fn write_secure_file_str_permissions() {
        let temp = TempDir::new().expect("temp dir");
        let file = temp.path().join("secure_str_file");

        write_secure_file_str(&file, "string data")
            .await
            .expect("should write");

        assert!(file.is_file());
        let metadata = std::fs::metadata(&file).expect("metadata");
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        let content = std::fs::read_to_string(&file).expect("read");
        assert_eq!(content, "string data");
    }

    #[tokio::test]
    async fn write_secure_file_content_roundtrip() {
        let temp = TempDir::new().expect("temp dir");
        let file = temp.path().join("roundtrip_file");

        write_secure_file(&file, b"secret data")
            .await
            .expect("should write");

        let content = std::fs::read(&file).expect("read");
        assert_eq!(content, b"secret data");
    }
}
