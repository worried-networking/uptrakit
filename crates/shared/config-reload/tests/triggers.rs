use std::io::Write;

use tempfile::NamedTempFile;
use tokio::sync::mpsc;
use uptrakit_config_reload::ReloadRequest;
use uptrakit_config_reload::triggers::file_watch::spawn_file_watch_task;
use uptrakit_config_reload::triggers::sighup::spawn_sighup_task;

#[tokio::test]
async fn sighup_task_forwards_signal_to_channel() {
    let (tx, mut rx) = mpsc::channel::<ReloadRequest>(8);
    let _task = spawn_sighup_task(tx);
    // Yield to the executor so the spawned task registers the OS signal handler
    // before we raise.
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    // Send SIGHUP to ourselves
    nix::sys::signal::raise(nix::sys::signal::Signal::SIGHUP).unwrap();
    let req = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("timeout")
        .expect("channel closed");
    drop(req);
}

#[tokio::test]
async fn file_watch_emits_request_after_atomic_rename() {
    let mut f = NamedTempFile::new().unwrap();
    writeln!(f, "initial").unwrap();
    let path = f.path().to_path_buf();
    let (tx, mut rx) = mpsc::channel::<ReloadRequest>(8);
    let _handle = spawn_file_watch_task(path.clone(), tx);

    // Yield to let the spawned task start and register the OS watcher before
    // we perform the rename.
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }

    // Atomic rename — simulate editor save.
    let mut other = NamedTempFile::new_in(path.parent().unwrap()).unwrap();
    writeln!(other, "updated").unwrap();
    std::fs::rename(other.path(), &path).unwrap();

    let req = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("timeout waiting for file-watch event")
        .expect("channel closed");
    drop(req);
}
