//! Workspace-standard transaction opening.
//!
//! `begin_immediate()` is the only sanctioned way to open a database
//! transaction in this workspace: it opens `BEGIN IMMEDIATE` on SQLite to
//! prevent `SQLITE_BUSY_SNAPSHOT` on read-then-write transactions and is a
//! mode no-op on other backends and on nested (savepoint) transactions. All
//! other transaction-opening methods are banned via `clippy.toml`
//! `disallowed-methods`.

use sea_orm::{
    DatabaseTransaction, DbErr, SqliteTransactionMode, TransactionOptions, TransactionTrait,
};

/// Opens a transaction with `BEGIN IMMEDIATE` on SQLite.
///
/// BEGIN IMMEDIATE prevents `SQLITE_BUSY_SNAPSHOT` on read-then-write
/// transactions (the write lock is taken at `BEGIN`, so `busy_timeout`
/// applies instead of an immediate snapshot error). On other backends and on
/// nested transactions (savepoints) the mode is silently ignored by SeaORM —
/// correct in both contexts.
#[expect(
    clippy::disallowed_methods,
    reason = "the workspace's sole sanctioned begin_with_options call site"
)]
pub async fn begin_immediate<C>(conn: &C) -> Result<DatabaseTransaction, DbErr>
where
    C: TransactionTrait<Transaction = DatabaseTransaction>,
{
    conn.begin_with_options(TransactionOptions {
        sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
        ..Default::default()
    })
    .await
}

#[cfg(test)]
mod tests {
    use sea_orm::{DatabaseConnection, TransactionTrait};

    /// Canary: proves the clippy `disallowed-methods` bans still resolve.
    ///
    /// If a sea-orm upgrade renames or relocates a banned method, the
    /// unresolvable `clippy.toml` path degrades to a config warning that
    /// `-D warnings` does not deny — but these expectations then go
    /// unfulfilled and `unfulfilled_lint_expectations = "deny"` fails the
    /// build instead.
    #[expect(dead_code, reason = "canary is never called")]
    async fn canary(db: &DatabaseConnection) {
        #[expect(
            clippy::disallowed_methods,
            reason = "canary: proves the TransactionTrait::begin ban still resolves"
        )]
        let _r1 = db.begin().await;
        #[expect(
            clippy::disallowed_methods,
            reason = "canary: proves the TransactionTrait::begin_with_options ban still resolves"
        )]
        let _r2 = db
            .begin_with_options(sea_orm::TransactionOptions::default())
            .await;
        #[expect(
            clippy::disallowed_methods,
            reason = "canary: proves the TransactionTrait::begin_with_config ban still resolves"
        )]
        let _r3 = db.begin_with_config(None, None).await;
    }
}

#[cfg(test)]
mod busy_snapshot_tests {
    // execute_unprepared is the approved raw-execution exception here: db-tx
    // defines no SeaORM entities; every statement is sea_query-built, never
    // string-concatenated.
    //
    // No #![expect(clippy::expect_used)] here (deviation from the brief's
    // verbatim snippet): clippy.toml's `allow-expect-in-tests = true`
    // already allows clippy::expect_used inside any #[cfg(test)] module in
    // this workspace, so the lint never fires here and the expectation would
    // be permanently unfulfilled — denied by `unfulfilled_lint_expectations
    // = "deny"` (workspace Cargo.toml). Confirmed via `cargo clippy
    // --all-targets --all-features`, which failed on this exact attribute
    // with "this lint expectation is unfulfilled" before removal.

    use std::time::Duration;

    use sea_orm::sea_query::{Alias, ColumnDef, Expr, ExprTrait, Query, SqliteQueryBuilder, Table};
    use sea_orm::{ConnectionTrait, DatabaseConnection, SqlxSqliteConnector, TransactionTrait};

    use crate::begin_immediate;

    async fn connect(path: &std::path::Path) -> DatabaseConnection {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_millis(100));
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .expect("connect sqlite");
        SqlxSqliteConnector::from_sqlx_sqlite_pool(pool)
    }

    async fn setup(path: &std::path::Path) -> (DatabaseConnection, DatabaseConnection) {
        let a = connect(path).await;
        let b = connect(path).await;
        let create = Table::create()
            .table(Alias::new("t"))
            .col(ColumnDef::new(Alias::new("v")).integer().not_null())
            .to_owned();
        a.execute_unprepared(&create.to_string(SqliteQueryBuilder))
            .await
            .expect("create table");
        (a, b)
    }

    fn insert_stmt() -> String {
        Query::insert()
            .into_table(Alias::new("t"))
            .columns([Alias::new("v")])
            .values_panic([1.into()])
            .to_owned()
            .to_string(SqliteQueryBuilder)
    }

    fn count_stmt() -> String {
        Query::select()
            .expr(Expr::col(Alias::new("v")).count())
            .from(Alias::new("t"))
            .to_owned()
            .to_string(SqliteQueryBuilder)
    }

    /// The bug class this crate exists to prevent: a DEFERRED read-then-write
    /// transaction whose snapshot goes stale errors with SQLITE_BUSY_SNAPSHOT
    /// immediately (busy_timeout does not apply).
    #[tokio::test]
    async fn deferred_read_then_write_fails_when_snapshot_goes_stale() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (a, b) = setup(&dir.path().join("t.db")).await;

        #[expect(
            clippy::disallowed_methods,
            reason = "negative control: reproduces the DEFERRED failure the helper prevents"
        )]
        let txa = a.begin().await.expect("deferred begin");
        txa.execute_unprepared(&count_stmt())
            .await
            .expect("read establishes the snapshot");
        b.execute_unprepared(&insert_stmt())
            .await
            .expect("concurrent commit invalidates the snapshot");
        let err = txa
            .execute_unprepared(&insert_stmt())
            .await
            .expect_err("upgrade write must fail with SQLITE_BUSY_SNAPSHOT");
        let msg = err.to_string();
        // 517 = SQLITE_BUSY_SNAPSHOT (extended code) — distinguishes the
        // stale-snapshot failure from a plain SQLITE_BUSY (5) timeout. The
        // "(code: N)" text is sqlx's Display format, asserted deliberately:
        // it fails loud in this leaf crate on a sqlx bump, and the message is
        // printed on failure.
        assert!(msg.contains("(code: 517)"), "unexpected error: {msg}");
    }

    /// With BEGIN IMMEDIATE the write lock is held from BEGIN, so no writer
    /// can invalidate the snapshot between the read and the write: the
    /// read-then-write transaction always completes.
    #[tokio::test]
    async fn immediate_serializes_writers_from_begin() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (a, b) = setup(&dir.path().join("t.db")).await;

        let txa = begin_immediate(&a).await.expect("immediate begin");
        txa.execute_unprepared(&count_stmt())
            .await
            .expect("read under the write lock");
        let busy = b
            .execute_unprepared(&insert_stmt())
            .await
            .expect_err("competing writer must wait (and here exhaust busy_timeout)");
        let busy_msg = busy.to_string();
        // 5 = plain SQLITE_BUSY: the writer queued behind the held lock and
        // exhausted busy_timeout — not a stale-snapshot failure.
        assert!(
            busy_msg.contains("(code: 5)"),
            "unexpected error: {busy_msg}"
        );
        txa.execute_unprepared(&insert_stmt())
            .await
            .expect("write after read succeeds — snapshot cannot be stale");
        txa.commit().await.expect("commit");
        b.execute_unprepared(&insert_stmt())
            .await
            .expect("competing writer succeeds once the lock is released");
    }
}
