//! Prints the compiled plugin catalog's contribution IDs as one JSON document
//! (ADR-0032, Layer B). Run by `cargo xtask contribution-monotonicity-check`
//! in two feature lanes; not built by a default `cargo build`.
//!
//! The `features` fingerprint must mirror this crate's `[features]` table in
//! `Cargo.toml` exactly — the xtask diffs the keys against `cargo metadata`
//! and fails on any missing/extra key, so drift here is a loud gate failure.

use std::process::ExitCode;

fn feature_fingerprint() -> serde_json::Value {
    serde_json::json!({
        "default": cfg!(feature = "default"),
        "daemon": cfg!(feature = "daemon"),
        "ssh": cfg!(feature = "ssh"),
        "migrations": cfg!(feature = "migrations"),
        "agent-infra": cfg!(feature = "agent-infra"),
        "dashboard-icons": cfg!(feature = "dashboard-icons"),
        "plugin-ops": cfg!(feature = "plugin-ops"),
        "notifications": cfg!(feature = "notifications"),
        "notifications-webhook": cfg!(feature = "notifications-webhook"),
        "notifications-telegram": cfg!(feature = "notifications-telegram"),
        "notifications-email": cfg!(feature = "notifications-email"),
        "test-support": cfg!(feature = "test-support"),
    })
}

fn main() -> ExitCode {
    let mut plugins = serde_json::Map::new();
    for desc in uptrakit_plugin_infrastructure_registry::all_descriptors() {
        let mut surfaces = serde_json::Map::new();
        if let Some(ops) = desc.surfaces {
            for registration in (ops.registrations)() {
                for surface in &registration.surfaces {
                    let interactions: Vec<serde_json::Value> = surface
                        .interactions
                        .iter()
                        .map(|i| {
                            let d = i.descriptor();
                            serde_json::json!([
                                d.interaction_id.as_str(),
                                d.effective_http_method().as_str(),
                            ])
                        })
                        .collect();
                    let data_sources: Vec<String> = surface
                        .data_sources
                        .iter()
                        .map(|ds| ds.data_source_id.as_str().to_string())
                        .collect();
                    surfaces.insert(
                        surface.descriptor.surface_id.as_str().to_string(),
                        serde_json::json!({
                            "interactions": interactions,
                            "data_sources": data_sources,
                        }),
                    );
                }
            }
        }
        let agent_interactions: Vec<String> = desc
            .agent_surfaces
            .map(|f| f().iter().map(|i| i.action_id.clone()).collect())
            .unwrap_or_default();
        // Positive cfg only (additive) — outside `migrations` the MigrationsFn
        // alias is a placeholder type, so only the wildcard arm exists and
        // these stay empty. A single `match` (not a `let mut` + `#[cfg]`
        // reassignment) avoids both an always-unused `mut` under the default
        // lane and an always-unused shadowed binding under this feature —
        // `MigrationTrait: MigrationName` is a supertrait, so `.name()` on
        // `Box<dyn MigrationTrait>` resolves without importing `MigrationName`
        // (an explicit `use` here would itself be unused; deny(warnings)).
        let migrations: Vec<String> = match cfg!(feature = "migrations") {
            #[cfg(feature = "migrations")]
            true => desc
                .migrations
                .map(|f| f().iter().map(|m| m.name().to_string()).collect())
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        let agent_migrations: Vec<String> = match cfg!(feature = "migrations") {
            #[cfg(feature = "migrations")]
            true => desc
                .agent_migrations
                .map(|f| f().iter().map(|m| m.name().to_string()).collect())
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        plugins.insert(
            desc.type_id.to_string(),
            serde_json::json!({
                "surfaces": surfaces,
                "agent_interactions": agent_interactions,
                "migrations": migrations,
                "agent_migrations": agent_migrations,
            }),
        );
    }
    let doc = serde_json::json!({
        "features": feature_fingerprint(),
        "plugins": plugins,
    });
    println!("{doc}");
    ExitCode::SUCCESS
}
