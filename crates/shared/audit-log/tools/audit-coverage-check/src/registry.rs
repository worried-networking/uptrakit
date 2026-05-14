//! Audit action registry loader.
//!
//! Parses `action_type.rs` from the `uptrakit-audit-log` crate and extracts
//! all `RegisteredAuditAction` constant declarations, classifying each as
//! [`Kind::Stateful`] or [`Kind::Event`].

use std::{collections::HashMap, path::Path};
use syn::{Expr, ExprCall, ExprLit, ExprPath, Item, Lit};

/// All registered audit actions keyed by their string value (e.g. `"auth.login"`).
#[derive(Debug)]
pub struct Registry {
    /// Map from action value string to its entry.
    pub actions: HashMap<String, RegistryEntry>,
}

/// Metadata for a single registered audit action.
#[derive(Debug, Clone)]
pub struct RegistryEntry {
    /// The Rust constant identifier as written in `action_type.rs`.
    pub const_ident: String,
    /// The runtime string value of the action (e.g. `"auth.login"`).
    pub value: String,
    /// Whether this action records before/after state or is an event-only record.
    pub kind: Kind,
}

/// Classification of an audit action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Kind {
    /// The action records a state transition (has before/after snapshots).
    Stateful,
    /// The action records a point-in-time event with no snapshot delta.
    Event,
}

/// Load the action registry by parsing the Rust source file at `path`.
///
/// Walks the AST of `action_type.rs` and extracts every `impl AuditActionType`
/// const that is initialised with `RegisteredAuditAction::new(...)`.
///
/// # Errors
///
/// Returns a descriptive string if the source file cannot be read or parsed.
pub fn load(path: &Path) -> Result<Registry, String> {
    let src = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let file: syn::File = syn::parse_str(&src).map_err(|e| e.to_string())?;
    let mut actions = HashMap::new();
    visit_items(&file.items, &mut actions);
    Ok(Registry { actions })
}

fn visit_items(items: &[Item], out: &mut HashMap<String, RegistryEntry>) {
    for item in items {
        match item {
            Item::Impl(imp) => {
                for it in &imp.items {
                    if let syn::ImplItem::Const(c) = it
                        && let Some(entry) = parse_registered(&c.ident.to_string(), &c.expr)
                    {
                        out.insert(entry.value.clone(), entry);
                    }
                }
            }
            Item::Mod(m) => {
                if let Some((_, items)) = &m.content {
                    visit_items(items, out);
                }
            }
            _ => {}
        }
    }
}

fn parse_registered(const_ident: &str, expr: &Expr) -> Option<RegistryEntry> {
    // Expect: RegisteredAuditAction::new("auth.login", AuditActionKind::Event)
    let Expr::Call(ExprCall { func, args, .. }) = expr else {
        return None;
    };
    let Expr::Path(ExprPath { path, .. }) = &**func else {
        return None;
    };
    if path.segments.last().is_none_or(|s| s.ident != "new") {
        return None;
    }
    if args.len() != 2 {
        return None;
    }
    let value = match args.first()? {
        Expr::Lit(ExprLit {
            lit: Lit::Str(s), ..
        }) => s.value(),
        _ => return None,
    };
    let kind = match args.iter().nth(1)? {
        Expr::Path(p) => {
            let last = p.path.segments.last()?;
            if last.ident == "Stateful" {
                Kind::Stateful
            } else if last.ident == "Event" {
                Kind::Event
            } else {
                return None;
            }
        }
        _ => return None,
    };
    Some(RegistryEntry {
        const_ident: const_ident.to_owned(),
        value,
        kind,
    })
}
