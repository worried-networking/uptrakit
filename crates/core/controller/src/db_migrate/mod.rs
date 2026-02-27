//! Database data migration — `uptrakit-controller db-migrate`.
//!
//! Reads all rows from a source database and bulk-inserts them into a target
//! database. The target schema is created via the normal `run_migrations` path
//! before any data is written.
//!
//! See [`run`] for the top-level entry point.

mod error;
mod tables;

pub use error::DbMigrateError;
use error::Result;

use std::io::Write as _;
use std::path::Path;
use std::time::Instant;

use rootcause::prelude::*;
use sea_orm::{EntityTrait, PaginatorTrait};

use crate::cli::{Args, DbMigrateArgs};

/// Run the `db-migrate` subcommand.
///
/// This function:
/// 1. Initializes the master encryption key (same as `serve` mode).
/// 2. Connects to `source_db` and `target_db`.
/// 3. Runs schema migrations on the target.
/// 4. Checks that the target has no user data (unless `--force`).
/// 5. Prompts for confirmation (unless `--yes`).
/// 6. Deletes all seeded rows from the target.
/// 7. Copies every application table from source to target.
/// 8. Verifies row counts match on both sides.
/// 9. Prints a summary.
pub async fn run(args: &Args, migrate_args: &DbMigrateArgs) -> Result<()> {
    let started = Instant::now();

    // ── 1. Master key ────────────────────────────────────────────────────────
    crate::startup::init_master_key(args)
        .map_err(|e| report!(DbMigrateError::Connection(format!("master key: {e}"))))?;

    // ── 2. Connect ───────────────────────────────────────────────────────────
    let src_url = &migrate_args.source_db;
    let dst_url = &migrate_args.target_db;

    // Validate URL schemes against enabled features before attempting connection.
    validate_url(src_url)
        .map_err(|e| report!(DbMigrateError::Connection(format!("source URL: {e}"))))?;
    validate_url(dst_url)
        .map_err(|e| report!(DbMigrateError::Connection(format!("target URL: {e}"))))?;

    eprintln!(
        "Connecting to source:  {}",
        crate::db::sanitize_url(src_url)
    );
    let src = crate::db::connect(src_url)
        .await
        .map_err(|e| report!(DbMigrateError::Connection(format!("source: {e}"))))?;

    eprintln!(
        "Connecting to target:  {}",
        crate::db::sanitize_url(dst_url)
    );
    let dst = crate::db::connect(dst_url)
        .await
        .map_err(|e| report!(DbMigrateError::Connection(format!("target: {e}"))))?;

    // ── 3. Schema migrations on target ───────────────────────────────────────
    eprintln!("Running schema migrations on target…");
    crate::migration::run_migrations(&dst)
        .await
        .map_err(|e| report!(DbMigrateError::TargetMigration(e.to_string())))?;

    // ── 4. Safety check ──────────────────────────────────────────────────────
    let user_count = uptrakit_shared_db::entity::prelude::User::find()
        .count(&dst)
        .await
        .map_err(|e| report!(DbMigrateError::Connection(format!("counting target users: {e}"))))?;

    if user_count > 0 && !migrate_args.force {
        bail!(DbMigrateError::TargetNotEmpty);
    }

    // ── 5. Confirmation prompt ───────────────────────────────────────────────
    if !migrate_args.yes {
        eprintln!();
        eprintln!(
            "WARNING: This will ERASE ALL DATA in the target database and replace it"
        );
        eprintln!("with data from the source database.");
        eprintln!();
        eprintln!("  Source: {}", crate::db::sanitize_url(src_url));
        eprintln!("  Target: {}", crate::db::sanitize_url(dst_url));
        eprintln!();
        eprint!("Type \"yes\" to continue: ");
        std::io::stderr().flush().ok();

        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .map_err(|e| {
                report!(DbMigrateError::Connection(format!("reading confirmation: {e}")))
            })?;

        if line.trim() != "yes" {
            bail!(DbMigrateError::MigrationAborted);
        }
    }

    // ── 6. Clean target ──────────────────────────────────────────────────────
    eprintln!("Cleaning target database…");
    tables::clean_all(&dst).await?;

    // ── 7. Copy tables ───────────────────────────────────────────────────────
    eprintln!("Copying tables…");
    let total_rows = tables::copy_all(&src, &dst, migrate_args.batch_size).await?;

    // ── 8. Verify ────────────────────────────────────────────────────────────
    eprintln!("Verifying row counts…");
    let verified = tables::verify_all(&src, &dst).await?;

    // Sanity-check: copy_all and verify_all should agree on totals.
    debug_assert_eq!(
        total_rows, verified,
        "copy total ({total_rows}) and verify total ({verified}) disagree"
    );

    // ── 9. Summary ───────────────────────────────────────────────────────────
    let elapsed = started.elapsed();
    eprintln!();
    eprintln!("Migration complete.");
    eprintln!("  Rows migrated : {total_rows}");
    eprintln!("  Elapsed       : {elapsed:.1?}");

    Ok(())
}

/// Validate that the URL scheme is supported by enabled features.
fn validate_url(url: &str) -> std::result::Result<(), String> {
    crate::db::DbConfig::from_args(Some(url.to_owned()), Path::new("/"))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use crate::cli::{Args, ControllerCommand};
    use clap::Parser;

    #[test]
    fn db_migrate_args_parse() {
        let args = Args::try_parse_from([
            "uptrakit-controller",
            "db-migrate",
            "--source-db",
            "sqlite::memory:",
            "--target-db",
            "sqlite::memory:",
        ])
        .expect("should parse db-migrate subcommand");

        let cmd = args.command.expect("command should be Some");
        match cmd {
            ControllerCommand::DbMigrate(ref db_args) => {
                assert_eq!(db_args.source_db, "sqlite::memory:");
                assert_eq!(db_args.target_db, "sqlite::memory:");
                assert_eq!(db_args.batch_size, 500);
                assert!(!db_args.force);
                assert!(!db_args.yes);
            }
        }
    }

    /// Integration test: migrate all seeded rows from one in-memory SQLite
    /// database to another and verify that row counts match exactly.
    ///
    /// Marked `#[ignore]` because it runs full schema migrations, which are
    /// slower than unit tests. Run with:
    ///
    /// ```text
    /// cargo test -p uptrakit-controller db_migrate -- --ignored
    /// ```
    #[tokio::test]
    #[ignore = "integration — runs schema migrations on two in-memory SQLite databases"]
    async fn migrate_sqlite_to_sqlite_roundtrip() {
        use sea_orm::{ConnectOptions, Database};

        // Set up source DB: run migrations so we get the seeded rows
        // (1 tenant, 3 roles, 9 permissions, role_permission links, scheduled tasks).
        let src = {
            let opt = ConnectOptions::new("sqlite::memory:");
            let db = Database::connect(opt).await.expect("source db");
            crate::migration::run_migrations(&db)
                .await
                .expect("source migrations");
            db
        };

        // Set up target DB: run migrations to create the schema (which also
        // inserts seed data), then clean it so we start with a blank slate.
        let dst = {
            let opt = ConnectOptions::new("sqlite::memory:");
            let db = Database::connect(opt).await.expect("target db");
            crate::migration::run_migrations(&db)
                .await
                .expect("target migrations");
            db
        };

        // Wipe the seeded rows from the target so `copy_all` starts clean.
        super::tables::clean_all(&dst).await.expect("clean_all");

        // Copy every table from source to target.
        let copied = super::tables::copy_all(&src, &dst, 500)
            .await
            .expect("copy_all");

        // Verify row counts agree on both sides.
        let verified = super::tables::verify_all(&src, &dst)
            .await
            .expect("verify_all");

        assert_eq!(
            copied, verified,
            "copy_all and verify_all must agree on the total row count"
        );
        assert!(
            copied > 0,
            "at least the migration-seeded rows should have been migrated"
        );
    }
}
