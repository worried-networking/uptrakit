// Re-export `TenantDb` from the shared-db crate so that callers importing
// from `uptrakit_web_api_queries` continue to work without changes.
pub use uptrakit_shared_db::TenantDb;
