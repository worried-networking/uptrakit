use std::sync::Arc;

use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection};

/// Set up a file-based SQLite database with all migrations applied.
///
/// Uses `tempfile` for a unique file path — each test gets its own database.
/// The file is automatically cleaned up when the returned `TempDir` is dropped.
pub(crate) async fn setup_sqlite() -> (
    DatabaseConnection,
    Option<Arc<dyn std::any::Any + Send + Sync>>,
) {
    let tmp_dir = tempfile::tempdir().expect("create temp dir for SQLite");
    let db_path = tmp_dir.path().join("test.db");
    let url = format!("sqlite://{}?mode=rwc", db_path.display());

    let opt = ConnectOptions::new(url);
    let db = Database::connect(opt).await.expect("connect to SQLite");

    uptrakit_shared_db::migration::run_migrations(&db)
        .await
        .expect("run SQLite migrations");

    // Keep TempDir alive so the file isn't deleted while tests run.
    let guard: Arc<dyn std::any::Any + Send + Sync> = Arc::new(tmp_dir);
    (db, Some(guard))
}

/// Set up a PostgreSQL database inside a testcontainers instance.
///
/// Each call starts a fresh container and creates a `test_{uuid}` database
/// with all migrations applied. The container is returned as a guard so it
/// is removed when the test function returns.
pub(crate) async fn setup_postgres() -> (
    DatabaseConnection,
    Option<Arc<dyn std::any::Any + Send + Sync>>,
) {
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;

    let container = Postgres::default()
        .with_tag("17-alpine")
        .start()
        .await
        .expect("start PostgreSQL container");
    let host_port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("get PG port");

    let db_name = format!("test_{}", uuid::Uuid::now_v7().simple());
    let admin_url = format!("postgres://postgres:postgres@127.0.0.1:{host_port}/postgres");
    let admin_opt = ConnectOptions::new(admin_url);
    let admin_db = Database::connect(admin_opt)
        .await
        .expect("connect to admin PG db");

    admin_db
        .execute_unprepared(&format!("CREATE DATABASE \"{db_name}\""))
        .await
        .expect("create test database");
    drop(admin_db);

    let url = format!("postgres://postgres:postgres@127.0.0.1:{host_port}/{db_name}");
    let opt = ConnectOptions::new(url);
    let db = Database::connect(opt).await.expect("connect to test PG db");

    uptrakit_shared_db::migration::run_migrations_debug(&db)
        .await
        .expect("run PG migrations");

    // Keep container alive for the duration of the test; Drop removes it.
    let guard: Arc<dyn std::any::Any + Send + Sync> = Arc::new(container);
    (db, Some(guard))
}

/// Set up a MariaDB database inside a testcontainers instance.
///
/// Each call starts a fresh container and creates a `test_{uuid}` database
/// with all migrations applied. The container is returned as a guard so it
/// is removed when the test function returns.
pub(crate) async fn setup_mariadb() -> (
    DatabaseConnection,
    Option<Arc<dyn std::any::Any + Send + Sync>>,
) {
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::mariadb::Mariadb;

    let container = Mariadb::default()
        .with_tag("11")
        .start()
        .await
        .expect("start MariaDB container");
    let host_port = container
        .get_host_port_ipv4(3306)
        .await
        .expect("get MariaDB port");

    let db_name = format!("test_{}", uuid::Uuid::now_v7().simple());
    let admin_url = format!("mysql://root@127.0.0.1:{host_port}/mysql");
    let admin_opt = ConnectOptions::new(admin_url);
    let admin_db = Database::connect(admin_opt)
        .await
        .expect("connect to admin MariaDB");

    admin_db
        .execute_unprepared(&format!("CREATE DATABASE `{db_name}`"))
        .await
        .expect("create test database");
    drop(admin_db);

    let url = format!("mysql://root@127.0.0.1:{host_port}/{db_name}");
    let opt = ConnectOptions::new(url);
    let db = Database::connect(opt)
        .await
        .expect("connect to test MariaDB db");

    uptrakit_shared_db::migration::run_migrations_debug(&db)
        .await
        .expect("run MariaDB migrations");

    // Keep container alive for the duration of the test; Drop removes it.
    let guard: Arc<dyn std::any::Any + Send + Sync> = Arc::new(container);
    (db, Some(guard))
}
