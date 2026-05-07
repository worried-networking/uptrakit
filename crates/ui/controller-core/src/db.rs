use sea_orm::DatabaseConnection;

/// Newtype wrapper for [`DatabaseConnection`] used as a focused Axum sub-state.
///
/// The inner field is private. External code accesses the connection via
/// [`DbState::db`]. Construction uses [`DbState::new`].
///
/// `#[non_exhaustive]` is applied so additional metadata fields (e.g., a read
/// replica connection) may be added in future versions without breaking
/// downstream pattern matches or struct literals.
///
/// # Examples
///
/// ```ignore
/// use sea_orm::Database;
/// use uptrakit_controller_core::db::DbState;
///
/// async fn example() {
///     let conn = Database::connect("sqlite::memory:").await.unwrap();
///     let state = DbState::new(conn);
///     let _db = state.db();
/// }
/// ```
#[non_exhaustive]
#[derive(Clone)]
pub struct DbState(DatabaseConnection);

impl DbState {
    /// Wraps a [`DatabaseConnection`] in [`DbState`].
    pub fn new(db: DatabaseConnection) -> Self {
        Self(db)
    }

    /// Returns a reference to the underlying database connection.
    pub fn db(&self) -> &DatabaseConnection {
        &self.0
    }
}

/// Implemented by application state types that hold a [`DbState`].
///
/// Allows [`DbState`] to implement [`axum::extract::FromRef<Arc<S>>`] for any
/// state type `S` that provides a database state — without violating Rust's
/// orphan rules.
///
/// Only available with the `axum-integration` feature.
///
/// # Examples
///
/// ```ignore
/// use std::sync::Arc;
/// use uptrakit_controller_core::db::{DbState, DbStateSource};
///
/// struct MyAppState {
///     db: DbState,
/// }
///
/// impl DbStateSource for MyAppState {
///     fn db_state(&self) -> DbState {
///         self.db.clone()
///     }
/// }
/// ```
#[cfg(feature = "axum-integration")]
pub trait DbStateSource {
    /// Returns a clone of the [`DbState`] held by this state.
    fn db_state(&self) -> DbState;
}

/// Enables Axum to extract [`DbState`] from any `Arc<S>` where `S: DbStateSource`.
///
/// This blanket implementation satisfies Axum's sub-state extraction for any
/// application state type that implements [`DbStateSource`]. The `Arc<S>`
/// wrapping matches the typical Axum router state pattern.
///
/// Only available with the `axum-integration` feature.
#[cfg(feature = "axum-integration")]
impl<S> axum::extract::FromRef<std::sync::Arc<S>> for DbState
where
    S: DbStateSource + Clone + Send + Sync + 'static,
{
    fn from_ref(state: &std::sync::Arc<S>) -> Self {
        state.db_state()
    }
}
