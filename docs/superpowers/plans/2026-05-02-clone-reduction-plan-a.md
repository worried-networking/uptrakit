# Clone Reduction — Plan A: web-api-queries

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans`
> to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate unnecessary heap allocations in batch-update query functions by iterating owned
maps by value instead of by reference, and by using `iter().copied()` on `Vec<Uuid>` passed to
SeaORM `is_in()`.

**Architecture:** Two mechanical patterns across `crates/ui/web-api-queries` and one file in
`crates/ui/web-api`. Pattern 1: change `for (id, x) in &found` to `for (id, x) in found` where
`found` is not accessed after the loop — moves the model into `ActiveModel` without cloning.
Pattern 2: replace `.is_in(v.clone())` with `.is_in(v.iter().copied())` for `Vec<Uuid>` values
(since `Uuid: Copy`), eliminating the `Vec` allocation on intermediate uses.

**Tech Stack:** Rust, SeaORM, `parking_lot`, `uuid`

---

## Task 1: Consume-by-value in `queries/hosts.rs`

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/hosts.rs:425-431`

Context: `batch_deactivate_hosts` iterates `&found` (a `HashMap<Uuid, host::Model>`), cloning
each model into an `ActiveModel`. The `contains_key` pass at lines 417-421 runs before the
loop — those borrows complete before the move, so consuming `found` is safe. `found` is not
accessed after the loop.

- [ ] **Step 1: Apply the change**

In `crates/ui/web-api-queries/src/queries/hosts.rs`, replace:

```rust
    for (id, h) in &found {
        let mut active: host::ActiveModel = h.clone().into();
        active.deactivated_at = Set(Some(now));
        active.updated_at = Set(now);
        active.update(tenant_db.db()).await?;
        succeeded.push(*id);
    }
```

with:

```rust
    for (id, h) in found {
        let mut active: host::ActiveModel = h.into();
        active.deactivated_at = Set(Some(now));
        active.updated_at = Set(now);
        active.update(tenant_db.db()).await?;
        succeeded.push(id);
    }
```

- [ ] **Step 2: Verify compilation**

```sh
cargo check -p uptrakit-web-api-queries --all-features
```

Expected: no errors.

- [ ] **Step 3: Commit**

```sh
git add crates/ui/web-api-queries/src/queries/hosts.rs
git commit -m "refactor(web-api-queries): consume host map by value in batch_deactivate_hosts"
```

---

## Task 2: Consume-by-value in `queries/services.rs`

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/services.rs` (three sites)

Three batch functions follow the same two-pass pattern: a `contains_key` loop first, then the
consuming loop. `found` is not accessed after any of the three loops.

- [ ] **Step 1: Apply changes to `batch_approve_services` (~line 552)**

Replace:

```rust
    for (id, svc) in &found {
        if svc.status != service::ServiceStatus::Pending {
            failed.push((*id, "service is not in pending status".to_string()));
            continue;
        }
        let mut active: service::ActiveModel = svc.clone().into();
        active.status = Set(service::ServiceStatus::Approved);
        active.updated_at = Set(now);
        active.update(tenant_db.db()).await.context_to()?;
        succeeded.push(*id);
    }
```

with:

```rust
    for (id, svc) in found {
        if svc.status != service::ServiceStatus::Pending {
            failed.push((id, "service is not in pending status".to_string()));
            continue;
        }
        let mut active: service::ActiveModel = svc.into();
        active.status = Set(service::ServiceStatus::Approved);
        active.updated_at = Set(now);
        active.update(tenant_db.db()).await.context_to()?;
        succeeded.push(id);
    }
```

- [ ] **Step 2: Apply changes to `batch_reject_services` (~line 746)**

Replace:

```rust
    for (id, svc) in &found {
        if svc.status != service::ServiceStatus::Pending {
            failed.push((*id, "service is not in pending status".to_string()));
            continue;
        }
        let mut active: service::ActiveModel = svc.clone().into();
        active.status = Set(service::ServiceStatus::Rejected);
        active.deactivated_at = Set(Some(now));
        active.updated_at = Set(now);
        active.update(tenant_db.db()).await.context_to()?;
        succeeded.push(*id);
    }
```

with:

```rust
    for (id, svc) in found {
        if svc.status != service::ServiceStatus::Pending {
            failed.push((id, "service is not in pending status".to_string()));
            continue;
        }
        let mut active: service::ActiveModel = svc.into();
        active.status = Set(service::ServiceStatus::Rejected);
        active.deactivated_at = Set(Some(now));
        active.updated_at = Set(now);
        active.update(tenant_db.db()).await.context_to()?;
        succeeded.push(id);
    }
```

- [ ] **Step 3: Apply changes to `batch_deactivate_services` (~line 797)**

Replace the entire loop body:

```rust
    // Deactivate each service in its own transaction so that individual
    // failures don't block the rest of the batch.
    for (id, svc) in &found {
        if svc.is_embedded {
            failed.push((*id, "embedded services cannot be deactivated".to_string()));
            continue;
        }

        let txn = tenant_db.db().begin().await.context_to()?;

        let mut active: service::ActiveModel = svc.clone().into();
        active.deactivated_at = Set(Some(now));
        active.updated_at = Set(now);
        active.update(&txn).await.context_to()?;

        ServiceCertificate::update_many()
            .col_expr(
                service_certificate::Column::RevokedAt,
                Expr::value(Some(now)),
            )
            .col_expr(
                service_certificate::Column::RevocationReason,
                Expr::value(Some(RevocationReason::ServiceDeactivated)),
            )
            .filter(service_certificate::Column::ServiceId.eq(*id))
            .filter(service_certificate::Column::RevokedAt.is_null())
            .exec(&txn)
            .await
            .context_to()?;

        crate::settings_version::bump_revocation_version(&txn, default_tenant_id)
            .await
            .map_err(|e| report!(ServiceQueryError::Db(e)))?;

        txn.commit().await.context_to()?;
        succeeded.push(*id);
    }
```

with:

```rust
    // Deactivate each service in its own transaction so that individual
    // failures don't block the rest of the batch.
    for (id, svc) in found {
        if svc.is_embedded {
            failed.push((id, "embedded services cannot be deactivated".to_string()));
            continue;
        }

        let txn = tenant_db.db().begin().await.context_to()?;

        let mut active: service::ActiveModel = svc.into();
        active.deactivated_at = Set(Some(now));
        active.updated_at = Set(now);
        active.update(&txn).await.context_to()?;

        ServiceCertificate::update_many()
            .col_expr(
                service_certificate::Column::RevokedAt,
                Expr::value(Some(now)),
            )
            .col_expr(
                service_certificate::Column::RevocationReason,
                Expr::value(Some(RevocationReason::ServiceDeactivated)),
            )
            .filter(service_certificate::Column::ServiceId.eq(id))
            .filter(service_certificate::Column::RevokedAt.is_null())
            .exec(&txn)
            .await
            .context_to()?;

        crate::settings_version::bump_revocation_version(&txn, default_tenant_id)
            .await
            .map_err(|e| report!(ServiceQueryError::Db(e)))?;

        txn.commit().await.context_to()?;
        succeeded.push(id);
    }
```

- [ ] **Step 4: Verify compilation**

```sh
cargo check -p uptrakit-web-api-queries --all-features
```

Expected: no errors.

- [ ] **Step 5: Commit**

```sh
git add crates/ui/web-api-queries/src/queries/services.rs
git commit -m "refactor(web-api-queries): consume service map by value in batch approve/reject/deactivate"
```

---

## Task 3: Consume-by-value in `queries/system_services.rs`

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/system_services.rs` (three sites ~393, 436, 480)

Same two-pass pattern as services.rs. `found` is not used after any loop.

- [ ] **Step 1: Apply change to `batch_approve_system_services` (~line 393)**

Replace:

```rust
    for (id, svc) in &found {
        if svc.status != SystemServiceStatus::Pending {
            failed.push((*id, "system service is not in pending status".to_string()));
            continue;
        }
        let mut active: system_service::ActiveModel = svc.clone().into();
        active.status = Set(SystemServiceStatus::Approved);
        active.updated_at = Set(now);
        active.update(db).await.context_to()?;
        succeeded.push(*id);
    }
```

with:

```rust
    for (id, svc) in found {
        if svc.status != SystemServiceStatus::Pending {
            failed.push((id, "system service is not in pending status".to_string()));
            continue;
        }
        let mut active: system_service::ActiveModel = svc.into();
        active.status = Set(SystemServiceStatus::Approved);
        active.updated_at = Set(now);
        active.update(db).await.context_to()?;
        succeeded.push(id);
    }
```

- [ ] **Step 2: Apply change to `batch_reject_system_services` (~line 436)**

Replace:

```rust
    for (id, svc) in &found {
        if svc.status != SystemServiceStatus::Pending {
            failed.push((*id, "system service is not in pending status".to_string()));
            continue;
        }
        let mut active: system_service::ActiveModel = svc.clone().into();
        active.status = Set(SystemServiceStatus::Rejected);
        active.deactivated_at = Set(Some(now));
        active.updated_at = Set(now);
        active.update(db).await.context_to()?;
        succeeded.push(*id);
    }
```

with:

```rust
    for (id, svc) in found {
        if svc.status != SystemServiceStatus::Pending {
            failed.push((id, "system service is not in pending status".to_string()));
            continue;
        }
        let mut active: system_service::ActiveModel = svc.into();
        active.status = Set(SystemServiceStatus::Rejected);
        active.deactivated_at = Set(Some(now));
        active.updated_at = Set(now);
        active.update(db).await.context_to()?;
        succeeded.push(id);
    }
```

- [ ] **Step 3: Apply change to `batch_deactivate_system_services` (~line 480)**

Replace the entire loop body:

```rust
    for (id, svc) in &found {
        if svc.is_embedded {
            failed.push((*id, "embedded services cannot be deactivated".to_string()));
            continue;
        }

        let txn = db.begin().await.context_to()?;

        let mut active: system_service::ActiveModel = svc.clone().into();
        active.deactivated_at = Set(Some(now));
        active.updated_at = Set(now);
        active.update(&txn).await.context_to()?;

        system_service_certificate::Entity::update_many()
            .col_expr(
                system_service_certificate::Column::RevokedAt,
                Expr::value(Some(now)),
            )
            .col_expr(
                system_service_certificate::Column::RevocationReason,
                Expr::value(Some(SystemRevocationReason::ServiceDeactivated)),
            )
            .filter(system_service_certificate::Column::SystemServiceId.eq(*id))
            .filter(system_service_certificate::Column::RevokedAt.is_null())
            .exec(&txn)
            .await
            .context_to()?;

        txn.commit().await.context_to()?;
        succeeded.push(*id);
    }
```

with:

```rust
    for (id, svc) in found {
        if svc.is_embedded {
            failed.push((id, "embedded services cannot be deactivated".to_string()));
            continue;
        }

        let txn = db.begin().await.context_to()?;

        let mut active: system_service::ActiveModel = svc.into();
        active.deactivated_at = Set(Some(now));
        active.updated_at = Set(now);
        active.update(&txn).await.context_to()?;

        system_service_certificate::Entity::update_many()
            .col_expr(
                system_service_certificate::Column::RevokedAt,
                Expr::value(Some(now)),
            )
            .col_expr(
                system_service_certificate::Column::RevocationReason,
                Expr::value(Some(SystemRevocationReason::ServiceDeactivated)),
            )
            .filter(system_service_certificate::Column::SystemServiceId.eq(id))
            .filter(system_service_certificate::Column::RevokedAt.is_null())
            .exec(&txn)
            .await
            .context_to()?;

        txn.commit().await.context_to()?;
        succeeded.push(id);
    }
```

- [ ] **Step 4: Verify compilation**

```sh
cargo check -p uptrakit-web-api-queries --all-features
```

Expected: no errors.

- [ ] **Step 5: Commit**

```sh
git add crates/ui/web-api-queries/src/queries/system_services.rs
git commit -m "refactor(web-api-queries): consume system_service map by value in batch approve/reject/deactivate"
```

---

## Task 4: Consume-by-value in remaining query files

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/plugin_configs.rs` (~line 291)
- Modify: `crates/ui/web-api-queries/src/queries/host_tags.rs` (~line 390)
- Modify: `crates/ui/web-api-queries/src/queries/software_items/crud.rs` (~lines 689, 729)

Note: `dispatch.rs:172` and `notifications.rs:149` are skipped — those models are accessed after
the clone site and cannot be consumed by value.

- [ ] **Step 1: Fix `plugin_configs.rs` (~line 291)**

Replace:

```rust
    for (id, config) in &found {
        let mut active: plugin_config::ActiveModel = config.clone().into();
        active.deactivated_at = Set(Some(now));
        active.enabled = Set(false);
        active.updated_at = Set(now);
        active.update(tenant_db.db()).await.context_to()?;
        succeeded.push(*id);
    }
```

with:

```rust
    for (id, config) in found {
        let mut active: plugin_config::ActiveModel = config.into();
        active.deactivated_at = Set(Some(now));
        active.enabled = Set(false);
        active.updated_at = Set(now);
        active.update(tenant_db.db()).await.context_to()?;
        succeeded.push(id);
    }
```

- [ ] **Step 2: Fix `host_tags.rs` (~line 390)**

Replace:

```rust
    for (id, tag) in &found {
        // Hard-delete assignments.
        host_tag_assignment::Entity::delete_many()
            .filter(host_tag_assignment::Column::HostTagId.eq(*id))
            .exec(&txn)
            .await?;

        // Soft-delete the tag.
        let mut active: host_tag::ActiveModel = tag.clone().into();
        active.deactivated_at = Set(Some(now));
        active.updated_at = Set(now);
        active.update(&txn).await?;
        succeeded.push(*id);
    }
```

with:

```rust
    for (id, tag) in found {
        // Hard-delete assignments.
        host_tag_assignment::Entity::delete_many()
            .filter(host_tag_assignment::Column::HostTagId.eq(id))
            .exec(&txn)
            .await?;

        // Soft-delete the tag.
        let mut active: host_tag::ActiveModel = tag.into();
        active.deactivated_at = Set(Some(now));
        active.updated_at = Set(now);
        active.update(&txn).await?;
        succeeded.push(id);
    }
```

- [ ] **Step 3: Fix `software_items/crud.rs` — `batch_feature_software_items` (~line 689)**

Replace:

```rust
    for (id, item) in &found {
        let mut active: software_item::ActiveModel = item.clone().into();
        active.featured = Set(true);
        active.updated_at = Set(now);
        active.update(tenant_db.db()).await.context_to()?;
        succeeded.push(*id);
    }
```

with:

```rust
    for (id, item) in found {
        let mut active: software_item::ActiveModel = item.into();
        active.featured = Set(true);
        active.updated_at = Set(now);
        active.update(tenant_db.db()).await.context_to()?;
        succeeded.push(id);
    }
```

- [ ] **Step 4: Fix `software_items/crud.rs` — `batch_delete_software_items` (~line 729)**

Replace:

```rust
    for (id, item) in &found {
        let mut active: software_item::ActiveModel = item.clone().into();
        active.deactivated_at = Set(Some(now));
        active.updated_at = Set(now);
        active.update(tenant_db.db()).await.context_to()?;
        succeeded.push(*id);
    }
```

with:

```rust
    for (id, item) in found {
        let mut active: software_item::ActiveModel = item.into();
        active.deactivated_at = Set(Some(now));
        active.updated_at = Set(now);
        active.update(tenant_db.db()).await.context_to()?;
        succeeded.push(id);
    }
```

- [ ] **Step 5: Verify compilation**

```sh
cargo check -p uptrakit-web-api-queries --all-features
```

Expected: no errors.

- [ ] **Step 6: Mid-point cross-feature check**

All consume-by-value changes in `web-api-queries` are complete after this task. Run the full
workspace check under both feature sets before continuing:

```sh
cargo check --no-default-features --features db-sqlite
cargo check --all-features
```

Expected: no errors. If any failure surfaces here, fix it before committing.

- [ ] **Step 7: Commit**

```sh
git add crates/ui/web-api-queries/src/queries/plugin_configs.rs \
        crates/ui/web-api-queries/src/queries/host_tags.rs \
        crates/ui/web-api-queries/src/queries/software_items/crud.rs
git commit -m "refactor(web-api-queries): consume model maps by value in plugin_configs, host_tags, software_items"
```

---

## Task 5: iter().copied() in `queries/software_states.rs`

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/software_states.rs` (lines 98, 136, 369, 404, 604)

`is_in` accepts `I: IntoIterator<Item = V>`. Passing `v.iter().copied()` for a `Vec<Uuid>` avoids
allocating a new `Vec` — it yields owned `Uuid` values (via `Copy`) directly. Use `.iter().copied()`
at all 5 sites. Optionally the bare owned form `.is_in(v)` can be used when `v` is not needed
after (the compiler would catch any mistake) — but `.iter().copied()` is correct for all sites.

- [ ] **Step 1: Apply changes**

At line 98, replace:

```rust
        .filter(host_software_item::Column::SoftwareItemId.is_in(item_ids.clone()))
```

with:

```rust
        .filter(host_software_item::Column::SoftwareItemId.is_in(item_ids.iter().copied()))
```

At line 136, replace:

```rust
        .filter(update_history::Column::SoftwareItemId.is_in(item_ids.clone()))
```

with:

```rust
        .filter(update_history::Column::SoftwareItemId.is_in(item_ids.iter().copied()))
```

At line 369, replace:

```rust
        .filter(host_software_item::Column::HostId.is_in(page_host_ids.clone()))
```

with:

```rust
        .filter(host_software_item::Column::HostId.is_in(page_host_ids.iter().copied()))
```

At line 404, replace:

```rust
        .filter(update_history::Column::HostId.is_in(page_host_ids.clone()))
```

with:

```rust
        .filter(update_history::Column::HostId.is_in(page_host_ids.iter().copied()))
```

At line 604, replace:

```rust
        .filter(host_tag_assignment::Column::HostId.is_in(host_ids.clone()))
```

with:

```rust
        .filter(host_tag_assignment::Column::HostId.is_in(host_ids.iter().copied()))
```

- [ ] **Step 2: Verify compilation**

```sh
cargo check -p uptrakit-web-api-queries --all-features
```

Expected: no errors.

- [ ] **Step 3: Commit**

```sh
git add crates/ui/web-api-queries/src/queries/software_states.rs
git commit -m "refactor(web-api-queries): use iter().copied() for Vec<Uuid> in software_states is_in filters"
```

---

## Task 6: iter().copied() in `queries/update_history.rs`

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/update_history.rs` (lines 117, 126)

`actor_ids` is used at lines 117, 126, and 134. Line 134 is already a bare last-use (no `.clone()`)
— leave it unchanged. Lines 117 and 126 are intermediate uses.

- [ ] **Step 1: Apply changes**

At line 117, replace:

```rust
        .filter(user::Column::Id.is_in(actor_ids.clone()))
```

with:

```rust
        .filter(user::Column::Id.is_in(actor_ids.iter().copied()))
```

At line 126, replace:

```rust
        .filter(service::Column::Id.is_in(actor_ids.clone()))
```

with:

```rust
        .filter(service::Column::Id.is_in(actor_ids.iter().copied()))
```

- [ ] **Step 2: Verify compilation**

```sh
cargo check -p uptrakit-web-api-queries --all-features
```

Expected: no errors.

- [ ] **Step 3: Commit**

```sh
git add crates/ui/web-api-queries/src/queries/update_history.rs
git commit -m "refactor(web-api-queries): use iter().copied() for Vec<Uuid> in update_history actor lookup"
```

---

## Task 7: iter().copied() in `routes/service_ws/handler/updates.rs`

**Files:**

- Modify: `crates/ui/web-api/src/routes/service_ws/handler/updates.rs` (lines 361, 382, 393, 407, 408, 472)

`host_ids` is used at lines 361, 393, 407, 472, and moved into the return value at line 482.
`sw_ids` is used at lines 382, 408, and as a bare last-use at line 473 (already no `.clone()`).
All `.clone()` calls in `is_in()` are intermediate uses — use `.iter().copied()` at all sites.

- [ ] **Step 1: Apply changes**

At line 361, replace:

```rust
        .filter(update_history::Column::HostId.is_in(host_ids.clone()))
```

with:

```rust
        .filter(update_history::Column::HostId.is_in(host_ids.iter().copied()))
```

At line 382, replace:

```rust
        .filter(software_item::Column::Id.is_in(sw_ids.clone()))
```

with:

```rust
        .filter(software_item::Column::Id.is_in(sw_ids.iter().copied()))
```

At line 393, replace:

```rust
        .filter(host::Column::Id.is_in(host_ids.clone()))
```

with:

```rust
        .filter(host::Column::Id.is_in(host_ids.iter().copied()))
```

At line 407, replace:

```rust
            .filter(host_software_item_plugin::Column::HostId.is_in(host_ids.clone()))
```

with:

```rust
            .filter(host_software_item_plugin::Column::HostId.is_in(host_ids.iter().copied()))
```

At line 408, replace:

```rust
            .filter(host_software_item_plugin::Column::SoftwareItemId.is_in(sw_ids.clone()))
```

with:

```rust
            .filter(host_software_item_plugin::Column::SoftwareItemId.is_in(sw_ids.iter().copied()))
```

At line 472, replace:

```rust
            .filter(host_software_item::Column::HostId.is_in(host_ids.clone()))
```

with:

```rust
            .filter(host_software_item::Column::HostId.is_in(host_ids.iter().copied()))
```

- [ ] **Step 2: Verify compilation**

```sh
cargo check -p uptrakit-web-api --all-features
```

Expected: no errors.

- [ ] **Step 3: Commit**

```sh
git add crates/ui/web-api/src/routes/service_ws/handler/updates.rs
git commit -m "refactor(web-api): use iter().copied() for Vec<Uuid> in pending-update batch queries"
```

---

## Task 8: Quality gates

- [ ] **Step 1: Format**

```sh
cargo fmt --all
```

- [ ] **Step 2: Full check (both feature sets)**

```sh
cargo check --no-default-features --features db-sqlite
cargo check --all-features
```

Expected: no errors.

- [ ] **Step 3: Clippy**

```sh
cargo clippy --all-targets --all-features
```

Expected: no new warnings.

- [ ] **Step 4: Tests**

```sh
cargo test -p uptrakit-web-api-queries --all-features
cargo test -p uptrakit-web-api --all-features
```

Expected: all pass.
