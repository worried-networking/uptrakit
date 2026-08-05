#![expect(
    clippy::expect_used,
    reason = "test helpers: panics on setup failure are acceptable"
)]

use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectOptions, Database, DatabaseConnection, EntityTrait,
    QueryFilter, Set,
};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{role, tenant, user, user_role};
use uptrakit_shared_types::MaskedEmail;
use uuid::Uuid;

use crate::queries::roles::{
    RoleNameCollision, RoleQueryError, create_role, delete_role_rows, find_role_name_collision,
    get_role, list_roles, update_role,
};

const BUILT_IN_ROLE_COUNT: usize = 8;

async fn test_db() -> DatabaseConnection {
    let mut opt = ConnectOptions::new("sqlite::memory:");
    opt.max_connections(1).min_connections(1);
    let db = Database::connect(opt).await.expect("connect to test db");
    uptrakit_shared_db::migration::run_migrations(&db)
        .await
        .expect("run migrations");
    db
}

async fn default_tenant_id(db: &DatabaseConnection) -> Uuid {
    tenant::Entity::find()
        .one(db)
        .await
        .expect("query tenants")
        .expect("default tenant is seeded")
        .id
}

/// Insert a second, non-default tenant row. `roles.tenant_id` and
/// `user_roles.tenant_id` both carry an FK to `tenants.id`, so a bare
/// `Uuid::now_v7()` "foreign tenant" would fail the FK — tests that need a
/// genuinely separate tenant bucket go through this.
async fn insert_tenant(db: &DatabaseConnection, slug: &str) -> Uuid {
    let id = Uuid::now_v7();
    let now = OffsetDateTime::now_utc();
    tenant::ActiveModel {
        id: Set(id),
        name: Set(slug.to_string()),
        slug: Set(slug.to_string()),
        is_default: Set(false),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    }
    .insert(db)
    .await
    .expect("insert second tenant");
    id
}

async fn role_id(db: &DatabaseConnection, name: &str) -> Uuid {
    role::Entity::find()
        .filter(role::Column::Name.eq(name))
        .one(db)
        .await
        .expect("query roles")
        .expect("seed role exists")
        .id
}

/// Insert an active user row; returns its id. Generates a UNIQUE email per
/// call — `users.email` carries `#[sea_orm(unique)]`.
async fn active_user(db: &DatabaseConnection) -> Uuid {
    let id = Uuid::now_v7();
    let now = OffsetDateTime::now_utc();
    user::ActiveModel {
        id: Set(id),
        email: Set(MaskedEmail::new(format!("{id}@roles.test"))),
        first_name: Set("Role".to_string()),
        last_name: Set("Holder".to_string()),
        password_hash: Set(None),
        is_active: Set(true),
        deactivated_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .expect("insert active user");
    id
}

/// Assign `role_id` to `user_id` in `tenant`.
async fn assign(db: &DatabaseConnection, tenant: Uuid, user_id: Uuid, role_id: Uuid) {
    user_role::ActiveModel {
        tenant_id: Set(tenant),
        user_id: Set(user_id),
        role_id: Set(role_id),
        assigned_at: Set(OffsetDateTime::now_utc()),
    }
    .insert(db)
    .await
    .expect("insert user_role assignment");
}

async fn user_role_count(db: &DatabaseConnection, role: Uuid) -> usize {
    user_role::Entity::find()
        .filter(user_role::Column::RoleId.eq(role))
        .all(db)
        .await
        .expect("query user_roles")
        .len()
}

#[tokio::test]
async fn list_includes_global_and_tenant_roles_get_scopes_correctly() {
    let db = test_db().await;
    let tenant_a = default_tenant_id(&db).await;
    let tenant_b = insert_tenant(&db, "foreign-tenant").await;

    let own_role = create_role(&db, tenant_a, "own-custom", None)
        .await
        .expect("create own custom role");
    let foreign_role = create_role(&db, tenant_b, "foreign-custom", None)
        .await
        .expect("create foreign custom role");

    let listed = list_roles(&db, tenant_a).await.expect("list roles");
    assert_eq!(
        listed.len(),
        BUILT_IN_ROLE_COUNT + 1,
        "must contain the 8 global built-ins plus exactly the tenant's own custom role"
    );
    assert!(
        listed.iter().any(|r| r.id == own_role.id),
        "own custom role must be listed"
    );
    assert!(
        listed.iter().all(|r| r.id != foreign_role.id),
        "foreign tenant's custom role must NOT be listed"
    );
    assert!(
        listed.iter().filter(|r| r.is_built_in).count() == BUILT_IN_ROLE_COUNT,
        "all 8 built-ins must be present"
    );

    // get scoping: own role resolves, foreign role is NotFound from tenant A's scope.
    let got = get_role(&db, tenant_a, own_role.id)
        .await
        .expect("get own role");
    assert_eq!(got.id, own_role.id);

    let err = get_role(&db, tenant_a, foreign_role.id)
        .await
        .expect_err("foreign role must not resolve in tenant A's scope");
    assert!(matches!(err.current_context(), RoleQueryError::NotFound));

    // Global built-ins resolve from any tenant's scope.
    let viewer_id = role_id(&db, "viewer").await;
    let got_viewer = get_role(&db, tenant_a, viewer_id)
        .await
        .expect("get global built-in from tenant A's scope");
    assert_eq!(got_viewer.id, viewer_id);
}

#[tokio::test]
async fn name_collision_detects_global_and_tenant_scopes_with_exclusion() {
    let db = test_db().await;
    let tenant_a = default_tenant_id(&db).await;

    let custom = create_role(&db, tenant_a, "deploy", None)
        .await
        .expect("create custom role");

    let global_hit = find_role_name_collision(&db, tenant_a, "viewer", None)
        .await
        .expect("query collision");
    assert_eq!(global_hit, Some(RoleNameCollision::Global));

    let tenant_hit = find_role_name_collision(&db, tenant_a, "deploy", None)
        .await
        .expect("query collision");
    assert_eq!(tenant_hit, Some(RoleNameCollision::Tenant));

    let self_exclusion = find_role_name_collision(&db, tenant_a, "deploy", Some(custom.id))
        .await
        .expect("query collision");
    assert_eq!(
        self_exclusion, None,
        "excluding the role being renamed must clear its own name collision"
    );

    let unknown = find_role_name_collision(&db, tenant_a, "does-not-exist", None)
        .await
        .expect("query collision");
    assert_eq!(unknown, None);
}

#[tokio::test]
async fn create_update_delete_roundtrip() {
    let db = test_db().await;
    let tenant_a = default_tenant_id(&db).await;
    let other_role_id = role_id(&db, "operator").await;

    let created = create_role(&db, tenant_a, "deploy", Some("Deploy role".to_string()))
        .await
        .expect("create role");
    assert!(!created.is_built_in);
    assert_eq!(created.tenant_id, Some(tenant_a));

    let (before, after) = update_role(
        &db,
        tenant_a,
        created.id,
        "deploy-renamed",
        Some("Updated description".to_string()),
    )
    .await
    .expect("update role");
    assert_eq!(before.id, created.id);
    assert_eq!(before.name, "deploy");
    assert_eq!(after.name, "deploy-renamed");
    assert_eq!(after.description, Some("Updated description".to_string()));
    assert_eq!(after.tenant_id, Some(tenant_a));
    assert_eq!(after.is_built_in, created.is_built_in);

    // Stage one assignment on the role being deleted, plus one on another
    // role, to prove deletion is scoped to the target role only.
    let user_id = active_user(&db).await;
    assign(&db, tenant_a, user_id, created.id).await;
    let other_user_id = active_user(&db).await;
    assign(&db, tenant_a, other_user_id, other_role_id).await;

    assert_eq!(user_role_count(&db, created.id).await, 1);
    assert_eq!(user_role_count(&db, other_role_id).await, 1);

    delete_role_rows(&db, tenant_a, created.id)
        .await
        .expect("delete role");

    assert_eq!(
        user_role_count(&db, created.id).await,
        0,
        "deleted role's assignments must be gone"
    );
    assert_eq!(
        user_role_count(&db, other_role_id).await,
        1,
        "other roles' assignments must be untouched"
    );

    let err = get_role(&db, tenant_a, created.id)
        .await
        .expect_err("deleted role must be gone");
    assert!(matches!(err.current_context(), RoleQueryError::NotFound));
}

#[tokio::test]
async fn mutations_reject_built_in_and_foreign_tenant_roles() {
    let db = test_db().await;
    let tenant_a = default_tenant_id(&db).await;
    let tenant_b = insert_tenant(&db, "foreign-tenant").await;

    let viewer_id = role_id(&db, "viewer").await;
    let viewer_before = get_role(&db, tenant_a, viewer_id)
        .await
        .expect("get viewer");

    let err = update_role(&db, tenant_a, viewer_id, "hijacked", None)
        .await
        .expect_err("update on a built-in must be rejected");
    assert!(matches!(err.current_context(), RoleQueryError::NotFound));

    let viewer_after = get_role(&db, tenant_a, viewer_id)
        .await
        .expect("get viewer again");
    assert_eq!(viewer_after.name, viewer_before.name);
    assert_eq!(viewer_after.description, viewer_before.description);

    let user_id = active_user(&db).await;
    assign(&db, tenant_a, user_id, viewer_id).await;
    assert_eq!(user_role_count(&db, viewer_id).await, 1);

    let err = delete_role_rows(&db, tenant_a, viewer_id)
        .await
        .expect_err("delete on a built-in must be rejected");
    assert!(matches!(err.current_context(), RoleQueryError::NotFound));

    // Role row and its assignment must be untouched.
    let viewer_still = get_role(&db, tenant_a, viewer_id)
        .await
        .expect("viewer still exists");
    assert_eq!(viewer_still.id, viewer_id);
    assert_eq!(user_role_count(&db, viewer_id).await, 1);

    // Foreign-tenant custom role: both mutation fns are NotFound from
    // tenant A's scope.
    let foreign_role = create_role(&db, tenant_b, "foreign-custom", None)
        .await
        .expect("create foreign custom role");

    let err = update_role(&db, tenant_a, foreign_role.id, "hijacked", None)
        .await
        .expect_err("update on a foreign-tenant role must be rejected");
    assert!(matches!(err.current_context(), RoleQueryError::NotFound));

    let err = delete_role_rows(&db, tenant_a, foreign_role.id)
        .await
        .expect_err("delete on a foreign-tenant role must be rejected");
    assert!(matches!(err.current_context(), RoleQueryError::NotFound));
}
