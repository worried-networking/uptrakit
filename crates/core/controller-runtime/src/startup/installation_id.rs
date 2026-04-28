use std::path::Path;

use rootcause::prelude::*;
use uuid::Uuid;

use crate::{AppError, Result};

const INSTALLATION_ID_FILE: &str = "controller-installation-id";

pub(crate) async fn init_installation_id(state_dir: &Path) -> Result<Uuid> {
    let path = state_dir.join(INSTALLATION_ID_FILE);

    match tokio::fs::read_to_string(&path).await {
        Ok(existing) => {
            let trimmed = existing.trim();
            return Uuid::parse_str(trimmed).map_err(|err| {
                report!(AppError::Config(format!(
                    "failed to parse controller installation ID at {}: {err}",
                    path.display()
                )))
            });
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(report!(AppError::Config(format!(
                "failed to read controller installation ID at {}: {err}",
                path.display()
            ))));
        }
    }

    let installation_id = Uuid::now_v7();
    uptrakit_directories::write_secure_file_str(&path, &installation_id.to_string())
        .await
        .map_err(|err| {
            report!(AppError::Config(format!(
                "failed to persist controller installation ID at {}: {err}",
                path.display()
            )))
        })?;
    Ok(installation_id)
}
