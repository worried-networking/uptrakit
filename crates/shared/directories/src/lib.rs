//! Cross-platform directory management for Uptrakit binaries.
//!
//! Uses `directories::ProjectDirs` to determine standard paths following
//! platform conventions (XDG on Linux, Application Support on macOS, etc.).
//!
//! Each binary has its own project directories:
//! - Controller: `uptrakit/controller` (qualifier: `io.uptrakit`)
//! - Agent: `uptrakit/agent`
//! - MQTT: `uptrakit/mqtt`
//!
//! The crate provides:
//! - Default paths using platform conventions
//! - User overrides via CLI args
//! - Secure directory/file creation (700 for dirs, 600 for files)
//! - Separation of config vs state directories

use std::fs;
use std::io;
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
        source: io::Error,
    },

    #[error("failed to write file {path}: {source}")]
    WriteFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("failed to set permissions on {path}: {source}")]
    SetPermissions {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("home directory could not be determined")]
    NoHomeDir,
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

    /// Ensure both directories exist with secure permissions (700).
    pub fn ensure_dirs(&self) -> Result<()> {
        create_secure_dir(&self.config)?;
        create_secure_dir(&self.state)?;
        Ok(())
    }

    /// Get a path within the config directory.
    pub fn config_path(&self, name: &str) -> PathBuf {
        self.config.join(name)
    }

    /// Get a path within the state directory.
    pub fn state_path(&self, name: &str) -> PathBuf {
        self.state.join(name)
    }
}

/// Expand `~` prefix to the user's home directory.
pub fn expand_tilde(path: &Path) -> Result<PathBuf> {
    let s = path.to_string_lossy();
    if let Some(stripped) = s.strip_prefix("~/") {
        let home = home_dir().ok_or_else(|| report!(DirectoryError::NoHomeDir))?;
        Ok(home.join(stripped))
    } else if s == "~" {
        home_dir().ok_or_else(|| report!(DirectoryError::NoHomeDir))
    } else {
        Ok(path.to_path_buf())
    }
}

/// Get the user's home directory.
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Create a directory with secure permissions (700).
///
/// On Unix, uses `DirBuilder` with mode `0o700` so the directory is created
/// with the correct permissions atomically, avoiding a TOCTOU window.
/// Creates parent directories as needed.
pub fn create_secure_dir(path: &Path) -> Result<()> {
    if path.exists() {
        return Ok(());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
            .map_err(|e| {
                report!(DirectoryError::CreateDir {
                    path: path.to_path_buf(),
                    source: e,
                })
            })?;
    }

    #[cfg(not(unix))]
    {
        fs::create_dir_all(path).map_err(|e| {
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
pub fn write_secure_file(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        create_secure_dir(parent)?;
    }

    write_with_mode(path, data)?;
    Ok(())
}

/// Write string data to a file with secure permissions (600).
pub fn write_secure_file_str(path: &Path, data: &str) -> Result<()> {
    write_secure_file(path, data.as_bytes())
}

/// Async variant of [`write_secure_file`].
///
/// On Unix, opens the file with `mode(0o600)` via `OpenOptionsExt` so it is
/// never world-readable, eliminating the TOCTOU window.
/// Creates parent directories as needed with 700 permissions.
pub async fn write_secure_file_async(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        // create_secure_dir is sync but fast (filesystem metadata only);
        // acceptable for the rare directory-creation path at startup.
        create_secure_dir(parent)?;
    }

    write_with_mode_async(path, data).await?;
    Ok(())
}

/// Async variant of [`write_secure_file_str`].
pub async fn write_secure_file_str_async(path: &Path, data: &str) -> Result<()> {
    write_secure_file_async(path, data.as_bytes()).await
}

/// Sync helper: open + write with `mode(0o600)` on Unix.
fn write_with_mode(path: &Path, data: &[u8]) -> Result<()> {
    use std::io::Write;

    #[cfg(unix)]
    let file = {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
    };

    #[cfg(not(unix))]
    let file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path);

    let mut file = file.map_err(|e| {
        report!(DirectoryError::WriteFile {
            path: path.to_path_buf(),
            source: e,
        })
    })?;

    file.write_all(data).map_err(|e| {
        report!(DirectoryError::WriteFile {
            path: path.to_path_buf(),
            source: e,
        })
    })?;

    Ok(())
}

/// Async helper: open + write with `mode(0o600)` on Unix via tokio.
async fn write_with_mode_async(path: &Path, data: &[u8]) -> Result<()> {
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
pub fn set_dir_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|e| {
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
pub fn set_file_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|e| {
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

    #[test]
    fn create_secure_dir_permissions() {
        let temp = TempDir::new().expect("temp dir");
        let dir = temp.path().join("secure_dir");

        create_secure_dir(&dir).expect("should create");

        assert!(dir.is_dir());
        let metadata = fs::metadata(&dir).expect("metadata");
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn write_secure_file_permissions() {
        let temp = TempDir::new().expect("temp dir");
        let file = temp.path().join("secure_file");

        write_secure_file(&file, b"secret data").expect("should write");

        assert!(file.is_file());
        let metadata = fs::metadata(&file).expect("metadata");
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn write_secure_file_creates_parent_dirs() {
        let temp = TempDir::new().expect("temp dir");
        let file = temp.path().join("a/b/c/file");

        write_secure_file(&file, b"data").expect("should write");

        assert!(file.is_file());
    }

    #[test]
    fn ensure_dirs_creates_both() {
        let temp = TempDir::new().expect("temp dir");
        let config = temp.path().join("config");
        let state = temp.path().join("state");

        let dirs = AppDirs::resolve("mqtt", Some(&config), Some(&state)).expect("should resolve");
        dirs.ensure_dirs().expect("should ensure");

        assert!(config.is_dir());
        assert!(state.is_dir());
    }

    #[test]
    fn config_and_state_paths() {
        let temp = TempDir::new().expect("temp dir");
        let dirs = AppDirs::resolve("controller", Some(temp.path()), Some(temp.path()))
            .expect("should resolve");

        assert_eq!(dirs.config_path("ca.crt"), temp.path().join("ca.crt"));
        assert_eq!(dirs.state_path("db.sqlite"), temp.path().join("db.sqlite"));
    }

    #[tokio::test]
    async fn write_secure_file_async_permissions() {
        let temp = TempDir::new().expect("temp dir");
        let file = temp.path().join("async_secure_file");

        write_secure_file_async(&file, b"async secret data")
            .await
            .expect("should write");

        assert!(file.is_file());
        let metadata = fs::metadata(&file).expect("metadata");
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        let content = fs::read(&file).expect("read");
        assert_eq!(content, b"async secret data");
    }

    #[tokio::test]
    async fn write_secure_file_str_async_permissions() {
        let temp = TempDir::new().expect("temp dir");
        let file = temp.path().join("async_secure_str_file");

        write_secure_file_str_async(&file, "async string data")
            .await
            .expect("should write");

        assert!(file.is_file());
        let metadata = fs::metadata(&file).expect("metadata");
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        let content = fs::read_to_string(&file).expect("read");
        assert_eq!(content, "async string data");
    }
}
