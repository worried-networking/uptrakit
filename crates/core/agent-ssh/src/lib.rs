//! SSH agent library for managing remote hosts over SSH.
//!
//! This crate can be used in two ways:
//!
//! - **Standalone binary** (`main.rs`): runs as a separate process connecting
//!   to the controller via WebSocket.
//! - **Embedded library**: the controller imports this crate and runs the SSH
//!   agent logic in-process using an `EmbeddedTransport`.

pub mod client;
pub mod db;
pub mod error;
pub mod extension;
pub mod host_ops;
pub mod ssh_pool;

pub(crate) mod host_info;
pub mod operations;
pub(crate) mod remote_exec;
pub(crate) mod ssh_executor;
pub mod ssh_key;
pub(crate) mod ssh_stdio_tunnel;
pub(crate) mod ssh_target;
pub(crate) mod ssh_transport;

use std::collections::{HashMap, HashSet};

/// AAD string for the `ssh_hosts.private_key` column.
pub const AAD_SSH_PRIVATE_KEY: &str = "uptrakit:ssh_hosts:private_key";

/// How often the daemon polls the local `ssh_hosts` table for changes.
pub const HOST_RELOAD_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

/// Minimum interval between consecutive update executions per host.
///
/// Rapid-fire update messages from a compromised controller are rejected
/// with a `security_audit:` warning. Legitimate orchestration always waits
/// for the previous update to finish before sending the next one.
pub const UPDATE_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(5);

// ---------------------------------------------------------------------------
// Encryption / key management helpers
// ---------------------------------------------------------------------------

/// Register the column AAD mapping for `ssh_hosts.private_key`.
///
/// Must be called after `init_master_key` and before any DB queries.
pub fn register_ssh_column_aad() {
    if !uptrakit_crypto::master_key_available() {
        return;
    }

    use uptrakit_crypto::ColumnAadEntry;

    let entries: &[ColumnAadEntry] = &[ColumnAadEntry {
        table: "ssh_hosts",
        column: "private_key",
        aad: AAD_SSH_PRIVATE_KEY,
    }];

    if let Err(e) = uptrakit_crypto::register_column_aad(entries) {
        tracing::warn!(error = %e, "column AAD registry already initialized (harmless)");
    }
}

/// Initialize the data key ring from the local DB (same pattern as controller).
pub async fn init_ssh_data_key_ring(db: &sea_orm::DatabaseConnection) {
    use sea_orm::{ActiveModelTrait, EntityTrait};

    if !uptrakit_crypto::master_key_available() {
        return;
    }

    let kek_fp = match uptrakit_crypto::master_key_fingerprint() {
        Ok(fp) => fp,
        Err(e) => {
            tracing::error!(error = %e, "failed to compute KEK fingerprint");
            return;
        }
    };

    let rows = match db::entity::data_encryption_key::Entity::find()
        .all(db)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "failed to query data_encryption_keys");
            return;
        }
    };

    if rows.is_empty() {
        // Generate the first DEK.
        let dek = match uptrakit_crypto::generate_data_key() {
            Ok(d) => d,
            Err(e) => {
                tracing::error!(error = %e, "failed to generate initial DEK");
                return;
            }
        };
        let wrapped = match uptrakit_crypto::wrap_data_key(&dek) {
            Ok(w) => w,
            Err(e) => {
                tracing::error!(error = %e, "failed to wrap initial DEK");
                return;
            }
        };

        let am = db::entity::data_encryption_key::ActiveModel {
            id: sea_orm::Set(uuid::Uuid::now_v7()),
            key_id: sea_orm::Set(dek.key_id.clone()),
            wrapped_key: sea_orm::Set(wrapped),
            kek_fingerprint: sea_orm::Set(kek_fp.clone()),
            status: sea_orm::Set("active".to_string()),
            created_at: sea_orm::Set(time::OffsetDateTime::now_utc()),
            retired_at: sea_orm::Set(None),
        };

        if let Err(e) = am.insert(db).await {
            tracing::debug!(error = %e, "initial DEK insert failed (may be race), will load existing");
        } else {
            tracing::info!(key_id = %dek.key_id, "generated initial data encryption key");
        }

        // Re-read in case of race.
        let rows = match db::entity::data_encryption_key::Entity::find()
            .all(db)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(error = %e, "failed to re-read data_encryption_keys");
                return;
            }
        };
        build_and_init_ssh_ring(&rows, &kek_fp);
        return;
    }

    build_and_init_ssh_ring(&rows, &kek_fp);
}

/// Build and init the data key ring from loaded DEK rows.
fn build_and_init_ssh_ring(rows: &[db::entity::data_encryption_key::Model], kek_fp: &str) {
    let mut keys = HashMap::new();
    let mut active_key_id: Option<String> = None;

    for row in rows {
        if row.kek_fingerprint != kek_fp {
            tracing::error!(
                key_id = %row.key_id,
                stored_fp = %row.kek_fingerprint,
                current_fp = %kek_fp,
                "DEK was wrapped with a different KEK — master key mismatch"
            );
            return;
        }

        let dek = match uptrakit_crypto::unwrap_data_key(&row.wrapped_key, &row.key_id) {
            Ok(d) => d,
            Err(e) => {
                tracing::error!(key_id = %row.key_id, error = %e, "failed to unwrap DEK");
                return;
            }
        };
        keys.insert(dek.key_id.clone(), dek.key);

        if row.status == "active" {
            active_key_id = Some(row.key_id.clone());
        }
    }

    let active = match active_key_id {
        Some(id) => id,
        None => {
            tracing::error!("no active DEK found in data_encryption_keys table");
            return;
        }
    };

    let ring = match uptrakit_crypto::DataKeyRing::new(keys, active.clone()) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "failed to construct data key ring");
            return;
        }
    };
    if let Err(e) = uptrakit_crypto::init_data_key_ring(ring) {
        tracing::warn!(error = %e, "data key ring already initialized (harmless)");
    } else {
        tracing::info!(active_key_id = %active, count = rows.len(), "data key ring initialized");
    }
}

/// Re-encrypt all non-v3 `ssh_hosts.private_key` values to `ENC:v3:`.
pub async fn reencrypt_ssh_to_v3(db: &sea_orm::DatabaseConnection) {
    use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel};
    use uptrakit_crypto::EncryptedString;

    if !uptrakit_crypto::master_key_available() {
        return;
    }

    let rows = match db::entity::ssh_host::Entity::find().all(db).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "failed to query ssh_hosts for v3 upgrade");
            return;
        }
    };

    let mut count = 0u64;
    for row in rows {
        if !row.private_key.needs_v3_upgrade() {
            continue;
        }
        let plaintext = row.private_key.expose_secret().to_string();
        let id = row.id;
        match EncryptedString::new(plaintext, AAD_SSH_PRIVATE_KEY) {
            Ok(encrypted) => {
                let mut am = row.into_active_model();
                am.private_key = sea_orm::Set(encrypted);
                if let Err(e) = am.update(db).await {
                    tracing::warn!(id = %id, error = %e, "v3 upgrade failed: ssh_hosts.private_key");
                } else {
                    count += 1;
                }
            }
            Err(e) => {
                tracing::warn!(id = %id, error = %e, "v3 encrypt failed: ssh_hosts.private_key");
            }
        }
    }
    if count > 0 {
        tracing::info!(
            table = "ssh_hosts",
            column = "private_key",
            count,
            "upgraded to ENC:v3"
        );
    }
}

// ---------------------------------------------------------------------------
// Snapshot diff helper
// ---------------------------------------------------------------------------

/// Compute the difference between two host snapshots.
///
/// Returns `(deleted_ids, changed_ids)`:
/// - `deleted_ids`: host IDs present in `prev` but absent from `curr`
/// - `changed_ids`: host IDs that are new in `curr`, or present in both but
///   with a different `updated_at`
pub fn diff_host_snapshots(
    prev: &[host_ops::HostSnapshot],
    curr: &[host_ops::HostSnapshot],
) -> (Vec<uuid::Uuid>, HashSet<uuid::Uuid>) {
    let prev_map: HashMap<uuid::Uuid, time::OffsetDateTime> =
        prev.iter().map(|s| (s.id, s.updated_at)).collect();
    let curr_ids: HashSet<uuid::Uuid> = curr.iter().map(|s| s.id).collect();

    let deleted: Vec<uuid::Uuid> = prev
        .iter()
        .filter(|s| !curr_ids.contains(&s.id))
        .map(|s| s.id)
        .collect();

    let mut changed: HashSet<uuid::Uuid> = HashSet::new();
    for snap in curr {
        match prev_map.get(&snap.id) {
            Some(&prev_ts) if prev_ts != snap.updated_at => {
                changed.insert(snap.id);
            }
            None => {
                // New host — needs SSH to discover machine_id.
                changed.insert(snap.id);
            }
            _ => {}
        }
    }

    (deleted, changed)
}

/// Handle a remote `SetUpdateFreeze` message by creating or removing the
/// freeze file on the local filesystem.
///
/// This piggybacks on the existing freeze-file mechanism: local `touch` still
/// works, and the freeze persists across agent restarts.
pub async fn handle_set_update_freeze(
    freeze_file_path: &std::path::Path,
    payload: uptrakit_internal_wire::SetUpdateFreezePayload,
) {
    let reason = payload.reason.as_deref().unwrap_or("(no reason given)");
    if payload.enabled {
        match tokio::fs::write(freeze_file_path, "").await {
            Ok(()) => {
                tracing::warn!(
                    freeze_file = %freeze_file_path.display(),
                    reason = reason,
                    "security_audit: update freeze enabled via remote command"
                );
            }
            Err(e) => {
                tracing::error!(
                    freeze_file = %freeze_file_path.display(),
                    error = %e,
                    "failed to create freeze file"
                );
            }
        }
    } else {
        match tokio::fs::remove_file(freeze_file_path).await {
            Ok(()) => {
                tracing::warn!(
                    freeze_file = %freeze_file_path.display(),
                    reason = reason,
                    "security_audit: update freeze disabled via remote command"
                );
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!(
                    freeze_file = %freeze_file_path.display(),
                    "freeze file did not exist; no action taken"
                );
            }
            Err(e) => {
                tracing::error!(
                    freeze_file = %freeze_file_path.display(),
                    error = %e,
                    "failed to remove freeze file"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_snapshots_no_change() {
        let a = vec![host_ops::HostSnapshot {
            id: uuid::Uuid::nil(),
            updated_at: time::OffsetDateTime::UNIX_EPOCH,
        }];
        let (del, changed) = diff_host_snapshots(&a, &a);
        assert!(del.is_empty());
        assert!(changed.is_empty());
    }

    #[test]
    fn diff_snapshots_added() {
        let prev = vec![];
        let curr = vec![host_ops::HostSnapshot {
            id: uuid::Uuid::nil(),
            updated_at: time::OffsetDateTime::UNIX_EPOCH,
        }];
        let (del, changed) = diff_host_snapshots(&prev, &curr);
        assert!(del.is_empty());
        assert!(changed.contains(&uuid::Uuid::nil()));
    }

    #[test]
    fn diff_snapshots_removed() {
        let prev = vec![host_ops::HostSnapshot {
            id: uuid::Uuid::nil(),
            updated_at: time::OffsetDateTime::UNIX_EPOCH,
        }];
        let curr = vec![];
        let (del, changed) = diff_host_snapshots(&prev, &curr);
        assert_eq!(del, vec![uuid::Uuid::nil()]);
        assert!(changed.is_empty());
    }

    #[test]
    fn diff_snapshots_updated() {
        let id = uuid::Uuid::nil();
        let prev = vec![host_ops::HostSnapshot {
            id,
            updated_at: time::OffsetDateTime::UNIX_EPOCH,
        }];
        let curr = vec![host_ops::HostSnapshot {
            id,
            updated_at: time::OffsetDateTime::UNIX_EPOCH + std::time::Duration::from_secs(1),
        }];
        let (del, changed) = diff_host_snapshots(&prev, &curr);
        assert!(del.is_empty());
        assert!(changed.contains(&id));
    }
}
