// Implemented in Task 14.
use std::path::PathBuf;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::coordinator::ReloadRequest;

/// Placeholder — full implementation in Task 14.
pub fn spawn_file_watch_task(
    _config_path: PathBuf,
    _tx: mpsc::Sender<ReloadRequest>,
) -> JoinHandle<()> {
    tokio::spawn(async {})
}
