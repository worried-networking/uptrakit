use super::error::{DbError, Result};
use rootcause::prelude::*;
use std::path::Path;

/// Database configuration
#[derive(Debug, Clone)]
pub struct DbConfig {
    pub url: String,
}

impl DbConfig {
    /// Create config from CLI args, defaulting to SQLite in data_dir
    #[cfg_attr(not(feature = "db-sqlite"), allow(unused_variables))]
    pub fn from_args(db_url: Option<String>, data_dir: &Path) -> Result<Self> {
        let url = match db_url {
            Some(url) => {
                // Validate URL scheme matches enabled features
                Self::validate_backend_support(&url)?;
                url
            }
            None => {
                // Default to SQLite
                #[cfg(not(feature = "db-sqlite"))]
                {
                    return Err(report!(DbError::Configuration(
                        "no database URL provided and SQLite feature not enabled".to_string(),
                    )));
                }

                #[cfg(feature = "db-sqlite")]
                {
                    let db_path = data_dir.join("uptrakit.db");
                    format!("sqlite://{}?mode=rwc", db_path.display())
                }
            }
        };

        Ok(Self { url })
    }

    /// Validate that the database URL scheme is supported by enabled features
    #[cfg_attr(
        not(any(feature = "db-sqlite", feature = "db-postgres", feature = "db-mysql")),
        allow(unreachable_code)
    )]
    fn validate_backend_support(url: &str) -> Result<()> {
        if url.starts_with("sqlite://") {
            #[cfg(not(feature = "db-sqlite"))]
            return Err(report!(DbError::Configuration(
                "SQLite URL provided but db-sqlite feature not enabled".to_string(),
            )));
        } else if url.starts_with("postgres://") || url.starts_with("postgresql://") {
            #[cfg(not(feature = "db-postgres"))]
            return Err(report!(DbError::Configuration(
                "PostgreSQL URL provided but db-postgres feature not enabled".to_string(),
            )));
        } else if url.starts_with("mysql://") {
            #[cfg(not(feature = "db-mysql"))]
            return Err(report!(DbError::Configuration(
                "MySQL URL provided but db-mysql feature not enabled".to_string(),
            )));
        } else {
            return Err(report!(DbError::Configuration(format!(
                "unsupported database URL scheme: {}",
                url.split("://").next().unwrap_or("unknown")
            ))));
        }

        Ok(())
    }
}
