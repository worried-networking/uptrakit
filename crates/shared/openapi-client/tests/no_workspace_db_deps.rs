//! Guardrail: assert that none of the five workspace-internal
//! database/encryption crates are reachable from this publishable crate's
//! resolve graph. See
//! docs/development/coding-standards.md "Publishable Crate Dependency Hygiene"
//! and docs/superpowers/specs/2026-06-09-publishable-crate-squat-chain-break-design.md.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;

use cargo_metadata::{CargoOpt, Metadata, MetadataCommand, PackageId};

const BANNED: &[&str] = &[
    "uptrakit-audit-log",
    "uptrakit-audit-log-derive",
    "uptrakit-shared-db",
    "uptrakit-tenant-db",
    "uptrakit-crypto",
];

/// Typed error surfaced by `find_banned_dep` when a banned crate is reachable
/// from the host package's resolve closure, or when the metadata is malformed.
/// Carries the chain string and is rendered by the test body's `panic!`.
enum FindBannedError {
    NoResolveGraph,
    HostNotInPackages,
    BannedReachable { name: String, chain: String },
}

impl fmt::Display for FindBannedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoResolveGraph => f.write_str("cargo metadata returned no resolve graph"),
            Self::HostNotInPackages => write!(
                f,
                "host crate `{}` not found in metadata.packages",
                env!("CARGO_PKG_NAME"),
            ),
            Self::BannedReachable { name, chain } => write!(
                f,
                "banned crate `{name}` reachable from `{host}`:\n  chain: {chain}",
                host = env!("CARGO_PKG_NAME"),
            ),
        }
    }
}

fn load_metadata(all_features: bool) -> Result<Metadata, cargo_metadata::Error> {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let mut cmd = MetadataCommand::new();
    cmd.manifest_path(manifest);
    if all_features {
        cmd.features(CargoOpt::AllFeatures);
    }
    cmd.exec()
}

/// Returns `Ok(())` when no banned crate is reachable from the host package's
/// resolve closure. Returns `Err(FindBannedError)` when a banned crate is hit
/// or the metadata cannot be walked.
fn find_banned_dep(metadata: &Metadata) -> Result<(), FindBannedError> {
    let host_pkg_name = env!("CARGO_PKG_NAME");

    let id_to_name: HashMap<&PackageId, &str> = metadata
        .packages
        .iter()
        .map(|p| (&p.id, p.name.as_ref()))
        .collect();

    let resolve = metadata
        .resolve
        .as_ref()
        .ok_or(FindBannedError::NoResolveGraph)?;

    let root: &PackageId = match resolve.root.as_ref() {
        Some(root) => root,
        None => match metadata
            .packages
            .iter()
            .find(|p| p.name.as_ref() == host_pkg_name)
        {
            Some(p) => &p.id,
            None => return Err(FindBannedError::HostNotInPackages),
        },
    };

    let node_children: HashMap<&PackageId, Vec<&PackageId>> = resolve
        .nodes
        .iter()
        .map(|n| (&n.id, n.deps.iter().map(|d| &d.pkg).collect()))
        .collect();

    let mut parents: HashMap<&PackageId, &PackageId> = HashMap::new();
    let mut visited: HashSet<&PackageId> = HashSet::from([root]);
    let mut queue: VecDeque<&PackageId> = VecDeque::from([root]);

    while let Some(current) = queue.pop_front() {
        if let Some(children) = node_children.get(current) {
            for child in children {
                if visited.insert(child) {
                    parents.insert(child, current);
                    queue.push_back(child);
                }
            }
        }
    }

    for id in &visited {
        let name = id_to_name.get(id).copied().unwrap_or("<unknown>");
        if BANNED.contains(&name) {
            let mut chain: Vec<&PackageId> = vec![id];
            let mut cursor: &PackageId = id;
            while let Some(parent) = parents.get(cursor) {
                chain.push(parent);
                cursor = parent;
            }
            chain.reverse();
            let chain_names: Vec<&str> = chain
                .iter()
                .map(|c| id_to_name.get(c).copied().unwrap_or("<unknown>"))
                .collect();
            return Err(FindBannedError::BannedReachable {
                name: name.to_string(),
                chain: chain_names.join(" -> "),
            });
        }
    }

    Ok(())
}

#[test]
fn no_workspace_db_deps() {
    let meta_default = load_metadata(false).expect("cargo metadata invocation failed");
    if let Err(err) = find_banned_dep(&meta_default) {
        panic!("[default-features] {err}");
    }

    let meta_all = load_metadata(true).expect("cargo metadata invocation failed");
    if let Err(err) = find_banned_dep(&meta_all) {
        panic!("[all-features] {err}");
    }
}
