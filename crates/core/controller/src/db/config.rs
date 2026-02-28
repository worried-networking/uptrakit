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
    pub fn from_args(db_url: Option<String>, data_dir: &Path) -> Result<Self> {
        let url = match db_url {
            Some(url) => {
                // Validate URL scheme matches enabled features
                Self::validate_backend_support(&url)?;
                url
            }
            None => {
                // Default to SQLite; error at runtime if the feature is not compiled in
                if !cfg!(feature = "db-sqlite") {
                    bail!(DbError::Configuration(
                        "no database URL provided and SQLite feature not enabled".to_string(),
                    ));
                }
                let db_path = data_dir.join("uptrakit.db");
                format!("sqlite://{}?mode=rwc", db_path.display())
            }
        };

        Ok(Self { url })
    }

    /// Validate that the database URL scheme is supported by enabled features
    fn validate_backend_support(url: &str) -> Result<()> {
        if url.starts_with("sqlite://") {
            if !cfg!(feature = "db-sqlite") {
                bail!(DbError::Configuration(
                    "SQLite URL provided but db-sqlite feature not enabled".to_string(),
                ));
            }
        } else if url.starts_with("postgres://") || url.starts_with("postgresql://") {
            if !cfg!(feature = "db-postgres") {
                bail!(DbError::Configuration(
                    "PostgreSQL URL provided but db-postgres feature not enabled".to_string(),
                ));
            }
        } else if url.starts_with("mysql://") {
            if !cfg!(feature = "db-mysql") {
                bail!(DbError::Configuration(
                    "MySQL URL provided but db-mysql feature not enabled".to_string(),
                ));
            }
        } else {
            bail!(DbError::Configuration(format!(
                "unsupported database URL scheme: {}",
                url.split("://").next().unwrap_or("unknown")
            )));
        }

        Ok(())
    }
}
