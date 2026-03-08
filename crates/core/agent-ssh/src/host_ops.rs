use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, Set,
};

use crate::db::entity::ssh_host::{ActiveModel, Column, Entity, Model, SshKeyType};
use crate::error::{Error, Result};

// ── HostSnapshot ──────────────────────────────────────────────────────────────

/// Lightweight representation of an `ssh_hosts` row used for change detection.
///
/// Only `id` and `updated_at` are stored — the full model is never cached in
/// memory across reload ticks.  The SSH agent polls these snapshots every
/// [`HOST_RELOAD_INTERVAL`](crate::HOST_RELOAD_INTERVAL) seconds to detect
/// additions, removals, and updates without loading full host credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostSnapshot {
    pub id: uuid::Uuid,
    pub updated_at: time::OffsetDateTime,
}

/// Return a lightweight snapshot of every row in `ssh_hosts`, ordered by `id`.
///
/// Used by `SshAgentHandler` to detect host-database changes without storing
/// full [`Model`] values in memory between reload ticks.
pub async fn list_host_snapshots(db: &DatabaseConnection) -> Result<Vec<HostSnapshot>> {
    let models = Entity::find()
        .order_by_asc(Column::Id)
        .all(db)
        .await
        .context_to::<Error>()?;
    Ok(models
        .into_iter()
        .map(|m| HostSnapshot {
            id: m.id,
            updated_at: m.updated_at,
        })
        .collect())
}

/// Parameters for adding a new SSH host.
pub struct AddHostParams {
    /// Pre-generated UUID for the new host entry.
    ///
    /// Callers must generate this before calling `add_host` so that the same
    /// ID can be embedded in other artefacts (e.g. the `authorized_keys`
    /// comment) before the DB row is created.
    pub host_id: uuid::Uuid,
    pub name: String,
    pub hostname: String,
    pub port: i32,
    pub username: String,
    pub encrypted_key: uptrakit_crypto::EncryptedString,
    pub key_type: SshKeyType,
    pub host_key_fingerprint: Option<String>,
}

/// Fields that can be updated on an SSH host.
pub struct HostUpdates {
    pub name: Option<String>,
    pub hostname: Option<String>,
    pub port: Option<i32>,
    pub username: Option<String>,
    pub private_key: Option<uptrakit_crypto::EncryptedString>,
    pub key_type: Option<SshKeyType>,
    pub host_key_fingerprint: Option<Option<String>>,
    /// Sudo policy string: `"auto"`, `"force_with"`, or `"force_without"`.
    /// When `Some`, replaces the stored sudo policy.
    pub sudo_policy: Option<String>,
}

/// Add a new SSH host to the database.
pub async fn add_host(db: &DatabaseConnection, params: AddHostParams) -> Result<Model> {
    // Check name uniqueness.
    let existing = Entity::find()
        .filter(Column::Name.eq(&params.name))
        .one(db)
        .await
        .context_to::<Error>()?;
    if existing.is_some() {
        bail!(Error::HostNameConflict(params.name));
    }

    let now = time::OffsetDateTime::now_utc();

    let model = ActiveModel {
        id: Set(params.host_id),
        name: Set(params.name),
        hostname: Set(params.hostname),
        port: Set(params.port),
        username: Set(params.username),
        private_key: Set(params.encrypted_key),
        key_type: Set(params.key_type),
        host_key_fingerprint: Set(params.host_key_fingerprint),
        machine_id: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        sudo_available: sea_orm::ActiveValue::NotSet,
        is_root: sea_orm::ActiveValue::NotSet,
        sudo_policy: Set("auto".to_string()),
        is_pve_node: Set(false),
        pve_plugin_config_id: Set(None),
        pve_node_name: Set(None),
    };

    let inserted = model.insert(db).await.context_to::<Error>()?;
    Ok(inserted)
}

/// Find an SSH host by name or UUID.
pub async fn find_host(db: &DatabaseConnection, name_or_id: &str) -> Result<Option<Model>> {
    // Try UUID parse first.
    if let Ok(uuid) = uuid::Uuid::try_parse(name_or_id) {
        let by_id = Entity::find_by_id(uuid)
            .one(db)
            .await
            .context_to::<Error>()?;
        if by_id.is_some() {
            return Ok(by_id);
        }
    }

    // Fall back to name lookup.
    Entity::find()
        .filter(Column::Name.eq(name_or_id))
        .one(db)
        .await
        .context_to::<Error>()
}

/// List all SSH hosts.
pub async fn list_hosts(db: &DatabaseConnection) -> Result<Vec<Model>> {
    Entity::find().all(db).await.context_to::<Error>()
}

/// List SSH hosts with pagination, ordered by name.
pub async fn list_hosts_paginated(
    db: &DatabaseConnection,
    page: u64,
    per_page: u64,
) -> Result<PaginatedHosts> {
    let base_query = Entity::find().order_by_asc(Column::Name);

    let total = base_query.clone().count(db).await.context_to::<Error>()?;

    let offset = (page.saturating_sub(1)) * per_page;
    let items = base_query
        .offset(Some(offset))
        .limit(Some(per_page))
        .all(db)
        .await
        .context_to::<Error>()?;

    let total_pages = if per_page == 0 {
        0
    } else {
        total.div_ceil(per_page)
    };

    Ok(PaginatedHosts {
        items,
        total,
        page,
        per_page,
        total_pages,
    })
}

/// Paginated list result for SSH hosts.
pub struct PaginatedHosts {
    pub items: Vec<Model>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
    pub total_pages: u64,
}

/// Remove an SSH host by name or UUID. Returns `true` if a row was deleted.
pub async fn remove_host(db: &DatabaseConnection, name_or_id: &str) -> Result<bool> {
    let host = find_host(db, name_or_id).await?;
    match host {
        Some(h) => {
            let model: ActiveModel = h.into();
            model.delete(db).await.context_to::<Error>()?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Update an SSH host by name or UUID.
pub async fn update_host(
    db: &DatabaseConnection,
    name_or_id: &str,
    updates: HostUpdates,
) -> Result<Model> {
    let host = find_host(db, name_or_id)
        .await?
        .ok_or_else(|| report!(Error::HostNotFound(name_or_id.to_string())))?;

    // If renaming, check uniqueness (excluding self).
    if let Some(ref new_name) = updates.name {
        let conflict = Entity::find()
            .filter(Column::Name.eq(new_name.as_str()))
            .one(db)
            .await
            .context_to::<Error>()?;
        if let Some(ref c) = conflict
            && c.id != host.id
        {
            bail!(Error::HostNameConflict(new_name.clone()));
        }
    }

    let mut model: ActiveModel = host.into();

    if let Some(name) = updates.name {
        model.name = Set(name);
    }
    if let Some(hostname) = updates.hostname {
        model.hostname = Set(hostname);
    }
    if let Some(port) = updates.port {
        model.port = Set(port);
    }
    if let Some(username) = updates.username {
        model.username = Set(username);
    }
    if let Some(key) = updates.private_key {
        model.private_key = Set(key);
    }
    if let Some(kt) = updates.key_type {
        model.key_type = Set(kt);
    }
    if let Some(fp) = updates.host_key_fingerprint {
        model.host_key_fingerprint = Set(fp);
    }
    if let Some(policy) = updates.sudo_policy {
        model.sudo_policy = Set(policy);
    }

    model.updated_at = Set(time::OffsetDateTime::now_utc());

    model.update(db).await.context_to::<Error>()
}

/// Update the `machine_id` for an SSH host identified by its UUID.
///
/// Called from `report_enrolled_hosts()` after the remote host's machine_id
/// is read, so that incoming `CheckVersions` / `ExecuteUpdate` messages can
/// be routed to the correct host via `find_host_by_machine_id()`.
pub async fn update_host_machine_id(
    db: &DatabaseConnection,
    host_id: uuid::Uuid,
    machine_id: &str,
) -> Result<()> {
    let host = Entity::find_by_id(host_id)
        .one(db)
        .await
        .context_to::<Error>()?
        .ok_or_else(|| report!(Error::HostNotFound(host_id.to_string())))?;

    let mut model: ActiveModel = host.into();
    model.machine_id = Set(Some(machine_id.to_string()));
    model.update(db).await.context_to::<Error>()?;
    Ok(())
}

/// Update the sudo state fields for an SSH host.
///
/// Called after sudo detection during bootstrap or `sync`.
/// Only fields with `Some` values are updated; `None` leaves the existing
/// value unchanged.
pub async fn update_host_sudo_state(
    db: &DatabaseConnection,
    host_id: uuid::Uuid,
    sudo_available: Option<bool>,
    is_root: Option<bool>,
    sudo_policy: Option<String>,
) -> Result<()> {
    let host = Entity::find_by_id(host_id)
        .one(db)
        .await
        .context_to::<Error>()?
        .ok_or_else(|| report!(Error::HostNotFound(host_id.to_string())))?;

    let mut model: ActiveModel = host.into();

    if let Some(v) = sudo_available {
        model.sudo_available = Set(Some(v));
    }
    if let Some(v) = is_root {
        model.is_root = Set(Some(v));
    }
    if let Some(policy) = sudo_policy {
        model.sudo_policy = Set(policy);
    }
    model.updated_at = Set(time::OffsetDateTime::now_utc());
    model.update(db).await.context_to::<Error>()?;
    Ok(())
}

/// Find an SSH host by its `machine_id`.
///
/// Returns `None` if no host with the given `machine_id` exists (including
/// hosts whose `machine_id` has not yet been populated and is `NULL`).
pub async fn find_host_by_machine_id(
    db: &DatabaseConnection,
    machine_id: &str,
) -> Result<Option<Model>> {
    Entity::find()
        .filter(Column::MachineId.eq(machine_id))
        .one(db)
        .await
        .context_to::<Error>()
}

/// Find SSH hosts by hostname, optionally narrowing by port.
///
/// Returns all matching hosts. When `port` is `Some`, only rows with that
/// port value are returned. Returns an empty `Vec` if there are no matches.
/// Used by the `sync` command to resolve an SSH address target to
/// a local DB entry.
pub async fn find_hosts_by_hostname(
    db: &DatabaseConnection,
    hostname: &str,
    port: Option<u16>,
) -> Result<Vec<Model>> {
    use sea_orm::QueryFilter;
    let mut query = Entity::find().filter(Column::Hostname.eq(hostname));
    if let Some(p) = port {
        query = query.filter(Column::Port.eq(i32::from(p)));
    }
    query.all(db).await.context_to::<Error>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_db;

    async fn setup_db() -> (tempfile::TempDir, DatabaseConnection) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = init_db(dir.path()).await.expect("init_db");
        (dir, db)
    }

    fn test_encrypted_key() -> uptrakit_crypto::EncryptedString {
        // Ensure a test master key is set (no-op if already initialized).
        let _ = uptrakit_crypto::init_master_key(zeroize::Zeroizing::new([0x42u8; 32]));
        // Register SSH column AAD so TryGetable can decrypt on read-back.
        let entries = &[uptrakit_crypto::ColumnAadEntry {
            table: "ssh_hosts",
            column: "private_key",
            aad: "uptrakit:ssh_hosts:private_key",
        }];
        let _ = uptrakit_crypto::register_column_aad(entries);
        uptrakit_crypto::EncryptedString::new(
            "test-key-content".to_string(),
            "uptrakit:ssh_hosts:private_key",
        )
        .expect("master key initialized above")
    }

    fn add_params(
        name: &str,
        hostname: &str,
        port: i32,
        username: &str,
        key_type: SshKeyType,
    ) -> AddHostParams {
        AddHostParams {
            host_id: uuid::Uuid::now_v7(),
            name: name.to_string(),
            hostname: hostname.to_string(),
            port,
            username: username.to_string(),
            encrypted_key: test_encrypted_key(),
            key_type,
            host_key_fingerprint: None,
        }
    }

    #[tokio::test]
    async fn add_and_find_host() {
        let (_dir, db) = setup_db().await;

        let host = add_host(
            &db,
            add_params("test-host", "192.168.1.1", 22, "root", SshKeyType::Ed25519),
        )
        .await
        .expect("add_host");

        assert_eq!(host.name, "test-host");
        assert_eq!(host.hostname, "192.168.1.1");
        assert_eq!(host.port, 22);

        // Find by name.
        let found = find_host(&db, "test-host")
            .await
            .expect("find_host")
            .expect("should exist");
        assert_eq!(found.id, host.id);

        // Find by ID.
        let found_by_id = find_host(&db, &host.id.to_string())
            .await
            .expect("find_host")
            .expect("should exist");
        assert_eq!(found_by_id.name, "test-host");
    }

    #[tokio::test]
    async fn add_duplicate_name_fails() {
        let (_dir, db) = setup_db().await;

        add_host(
            &db,
            add_params("dup", "host1", 22, "user1", SshKeyType::Ed25519),
        )
        .await
        .expect("first add");

        let result = add_host(
            &db,
            add_params("dup", "host2", 22, "user2", SshKeyType::Rsa),
        )
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err.current_context(), Error::HostNameConflict(name) if name == "dup"),
            "expected HostNameConflict, got: {err}"
        );
    }

    #[tokio::test]
    async fn list_hosts_empty_and_populated() {
        let (_dir, db) = setup_db().await;

        let list = list_hosts(&db).await.expect("list");
        assert!(list.is_empty());

        add_host(
            &db,
            add_params("h1", "host1", 22, "user", SshKeyType::Ed25519),
        )
        .await
        .expect("add");

        add_host(
            &db,
            add_params("h2", "host2", 2222, "user", SshKeyType::Rsa),
        )
        .await
        .expect("add");

        let list = list_hosts(&db).await.expect("list");
        assert_eq!(list.len(), 2);
    }

    #[tokio::test]
    async fn remove_host_by_name() {
        let (_dir, db) = setup_db().await;

        add_host(
            &db,
            add_params("to-remove", "host1", 22, "user", SshKeyType::Ed25519),
        )
        .await
        .expect("add");

        let removed = remove_host(&db, "to-remove").await.expect("remove");
        assert!(removed);

        let found = find_host(&db, "to-remove").await.expect("find");
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn remove_nonexistent_returns_false() {
        let (_dir, db) = setup_db().await;
        let removed = remove_host(&db, "nope").await.expect("remove");
        assert!(!removed);
    }

    #[tokio::test]
    async fn update_host_fields() {
        let (_dir, db) = setup_db().await;

        add_host(
            &db,
            add_params("updatable", "host1", 22, "user", SshKeyType::Ed25519),
        )
        .await
        .expect("add");

        let updated = update_host(
            &db,
            "updatable",
            HostUpdates {
                name: None,
                hostname: Some("new-host".to_string()),
                port: Some(2222),
                username: None,
                private_key: None,
                key_type: None,
                host_key_fingerprint: Some(Some("SHA256:abc123".to_string())),
                sudo_policy: None,
            },
        )
        .await
        .expect("update");

        assert_eq!(updated.hostname, "new-host");
        assert_eq!(updated.port, 2222);
        assert_eq!(
            updated.host_key_fingerprint.as_deref(),
            Some("SHA256:abc123")
        );
    }

    #[tokio::test]
    async fn update_rename_conflict() {
        let (_dir, db) = setup_db().await;

        add_host(&db, add_params("host-a", "a", 22, "u", SshKeyType::Ed25519))
            .await
            .expect("add a");

        add_host(&db, add_params("host-b", "b", 22, "u", SshKeyType::Ed25519))
            .await
            .expect("add b");

        let result = update_host(
            &db,
            "host-a",
            HostUpdates {
                name: Some("host-b".to_string()),
                hostname: None,
                port: None,
                username: None,
                private_key: None,
                key_type: None,
                host_key_fingerprint: None,
                sudo_policy: None,
            },
        )
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err.current_context(), Error::HostNameConflict(name) if name == "host-b"),
            "expected HostNameConflict, got: {err}"
        );
    }

    #[tokio::test]
    async fn update_nonexistent_fails() {
        let (_dir, db) = setup_db().await;

        let result = update_host(
            &db,
            "ghost",
            HostUpdates {
                name: None,
                hostname: None,
                port: None,
                username: None,
                private_key: None,
                key_type: None,
                host_key_fingerprint: None,
                sudo_policy: None,
            },
        )
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err.current_context(), Error::HostNotFound(_)));
    }

    #[tokio::test]
    async fn rename_to_same_name_succeeds() {
        let (_dir, db) = setup_db().await;

        add_host(
            &db,
            add_params("same-name", "h", 22, "u", SshKeyType::Ed25519),
        )
        .await
        .expect("add");

        let updated = update_host(
            &db,
            "same-name",
            HostUpdates {
                name: Some("same-name".to_string()),
                hostname: None,
                port: None,
                username: None,
                private_key: None,
                key_type: None,
                host_key_fingerprint: None,
                sudo_policy: None,
            },
        )
        .await
        .expect("rename to self should succeed");

        assert_eq!(updated.name, "same-name");
    }

    // ── machine_id tests ────────────────────────────────────────────────────

    #[tokio::test]
    async fn update_host_machine_id_sets_and_find_by_machine_id_returns_host() {
        let (_dir, db) = setup_db().await;

        let host = add_host(
            &db,
            add_params("target", "192.168.1.10", 22, "root", SshKeyType::Ed25519),
        )
        .await
        .expect("add_host");

        // Initially machine_id is None, so find_host_by_machine_id returns None.
        let not_found = find_host_by_machine_id(&db, "abc123")
            .await
            .expect("find_host_by_machine_id");
        assert!(not_found.is_none());

        // Set the machine_id.
        update_host_machine_id(&db, host.id, "abc123")
            .await
            .expect("update_host_machine_id");

        // Now find_host_by_machine_id should return the host.
        let found = find_host_by_machine_id(&db, "abc123")
            .await
            .expect("find_host_by_machine_id")
            .expect("host should be found");
        assert_eq!(found.id, host.id);
        assert_eq!(found.machine_id, Some("abc123".to_string()));
    }

    #[tokio::test]
    async fn find_host_by_machine_id_empty_string_returns_none() {
        let (_dir, db) = setup_db().await;
        // machine_id is stored as NULL until populated; no row has an empty string.
        let result = find_host_by_machine_id(&db, "")
            .await
            .expect("find_host_by_machine_id");
        assert!(result.is_none());
    }

    /// Regression test: `update_host_machine_id()` must NOT bump `updated_at`.
    ///
    /// The `machine_id` is runtime-discovered metadata (read from the remote
    /// host's `/etc/machine-id`), not user configuration. Bumping `updated_at`
    /// here caused the dynamic-reload loop to re-detect the host as "changed"
    /// every tick, leading to infinite reconnect cycles.
    #[tokio::test]
    async fn update_host_machine_id_does_not_change_updated_at() {
        let (_dir, db) = setup_db().await;

        let host = add_host(
            &db,
            add_params("mid-ts", "10.0.0.50", 22, "root", SshKeyType::Ed25519),
        )
        .await
        .expect("add_host");

        let before = list_host_snapshots(&db)
            .await
            .expect("snapshots")
            .into_iter()
            .find(|s| s.id == host.id)
            .expect("snapshot present");

        // Set machine_id — this must NOT change updated_at.
        update_host_machine_id(&db, host.id, "machine-id-value")
            .await
            .expect("update_host_machine_id");

        let after = list_host_snapshots(&db)
            .await
            .expect("snapshots")
            .into_iter()
            .find(|s| s.id == host.id)
            .expect("snapshot present");

        assert_eq!(
            before.updated_at, after.updated_at,
            "update_host_machine_id() must not bump updated_at — it is runtime metadata, \
             not user configuration; bumping it causes infinite reload loops"
        );
    }

    #[tokio::test]
    async fn update_host_machine_id_nonexistent_id_fails() {
        let (_dir, db) = setup_db().await;
        let result = update_host_machine_id(&db, uuid::Uuid::nil(), "mid").await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err().current_context(),
            Error::HostNotFound(_)
        ));
    }

    // ── find_hosts_by_hostname tests ────────────────────────────────────────

    #[tokio::test]
    async fn find_hosts_by_hostname_returns_matching_host() {
        let (_dir, db) = setup_db().await;

        add_host(
            &db,
            add_params("myhost", "10.0.0.1", 22, "uptrakit", SshKeyType::Ed25519),
        )
        .await
        .expect("add");

        let results = find_hosts_by_hostname(&db, "10.0.0.1", None)
            .await
            .expect("find");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "myhost");
    }

    #[tokio::test]
    async fn find_hosts_by_hostname_with_port_filter() {
        let (_dir, db) = setup_db().await;

        add_host(
            &db,
            add_params(
                "host-22",
                "srv.example.com",
                22,
                "uptrakit",
                SshKeyType::Ed25519,
            ),
        )
        .await
        .expect("add host-22");
        add_host(
            &db,
            add_params(
                "host-2222",
                "srv.example.com",
                2222,
                "uptrakit",
                SshKeyType::Ed25519,
            ),
        )
        .await
        .expect("add host-2222");

        let results = find_hosts_by_hostname(&db, "srv.example.com", Some(2222))
            .await
            .expect("find");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "host-2222");
    }

    #[tokio::test]
    async fn find_hosts_by_hostname_no_match_returns_empty() {
        let (_dir, db) = setup_db().await;

        let results = find_hosts_by_hostname(&db, "nonexistent.example.com", None)
            .await
            .expect("find");
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn find_hosts_by_hostname_multiple_matches_without_port_filter() {
        let (_dir, db) = setup_db().await;

        add_host(
            &db,
            add_params("h1", "shared.host", 22, "u1", SshKeyType::Ed25519),
        )
        .await
        .expect("add h1");
        add_host(
            &db,
            add_params("h2", "shared.host", 2222, "u2", SshKeyType::Ed25519),
        )
        .await
        .expect("add h2");

        let results = find_hosts_by_hostname(&db, "shared.host", None)
            .await
            .expect("find");
        assert_eq!(results.len(), 2);
    }

    // ── list_host_snapshots tests ────────────────────────────────────────────

    #[tokio::test]
    async fn list_host_snapshots_empty() {
        let (_dir, db) = setup_db().await;
        let snapshots = list_host_snapshots(&db).await.expect("list_host_snapshots");
        assert!(snapshots.is_empty());
    }

    #[tokio::test]
    async fn list_host_snapshots_returns_id_and_updated_at() {
        let (_dir, db) = setup_db().await;

        let h1 = add_host(
            &db,
            add_params("snap-a", "10.0.0.1", 22, "u", SshKeyType::Ed25519),
        )
        .await
        .expect("add h1");

        let h2 = add_host(
            &db,
            add_params("snap-b", "10.0.0.2", 22, "u", SshKeyType::Ed25519),
        )
        .await
        .expect("add h2");

        let snapshots = list_host_snapshots(&db).await.expect("list_host_snapshots");
        assert_eq!(snapshots.len(), 2);

        // Ordered by id.
        let ids: Vec<uuid::Uuid> = snapshots.iter().map(|s| s.id).collect();
        assert!(ids.contains(&h1.id));
        assert!(ids.contains(&h2.id));

        for snap in &snapshots {
            assert!(snap.updated_at > time::OffsetDateTime::UNIX_EPOCH);
        }
    }

    #[tokio::test]
    async fn list_host_snapshots_reflects_update() {
        let (_dir, db) = setup_db().await;

        let host = add_host(
            &db,
            add_params("snap-update", "10.0.0.3", 22, "u", SshKeyType::Ed25519),
        )
        .await
        .expect("add");

        let before = list_host_snapshots(&db)
            .await
            .expect("before")
            .into_iter()
            .find(|s| s.id == host.id)
            .expect("snapshot present");

        // Small sleep to ensure updated_at changes (it has 1-second resolution).
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        update_host(
            &db,
            "snap-update",
            HostUpdates {
                name: Some("snap-updated".to_string()),
                hostname: None,
                port: None,
                username: None,
                private_key: None,
                key_type: None,
                host_key_fingerprint: None,
                sudo_policy: None,
            },
        )
        .await
        .expect("update");

        let after = list_host_snapshots(&db)
            .await
            .expect("after")
            .into_iter()
            .find(|s| s.id == host.id)
            .expect("snapshot present");

        assert!(
            after.updated_at >= before.updated_at,
            "updated_at should not decrease"
        );
    }
}
