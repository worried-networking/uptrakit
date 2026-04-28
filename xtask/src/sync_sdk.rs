use anyhow::Result;
use std::path::Path;

pub fn run(workspace_root: &Path, check: bool) -> Result<()> {
    println!("sync-sdk: workspace root = {}", workspace_root.display());
    if check {
        println!("sync-sdk: --check mode (no-op for now)");
    }
    Ok(())
}
