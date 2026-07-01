//! Parse the hand-written client source: method names + path templates.

use std::path::Path;
use syn::visit::{self, Visit};

/// Collect `pub async fn` names declared in `impl UptrakitClient` (non-trait) blocks.
#[must_use]
pub fn methods_in_source(src: &str) -> Vec<String> {
    let Ok(file) = syn::parse_file(src) else {
        return Vec::new();
    };
    let mut c = MethodCollector::default();
    c.visit_file(&file);
    c.methods
}

#[derive(Default)]
struct MethodCollector {
    methods: Vec<String>,
}

impl<'ast> Visit<'ast> for MethodCollector {
    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        let is_client = node.trait_.is_none()
            && matches!(&*node.self_ty, syn::Type::Path(tp)
                if tp.path.segments.last().is_some_and(|s| s.ident == "UptrakitClient"));
        if is_client {
            for item in &node.items {
                if let syn::ImplItem::Fn(f) = item
                    && matches!(f.vis, syn::Visibility::Public(_))
                    && f.sig.asyncness.is_some()
                {
                    self.methods.push(f.sig.ident.to_string());
                }
            }
        }
        visit::visit_item_impl(self, node);
    }
}

/// Walk `client_src_dir` for `*.rs` and union all client method names.
///
/// # Errors
/// Returns an error string if the directory cannot be walked or a file cannot be read.
pub fn collect_methods(client_src_dir: &Path) -> Result<Vec<String>, String> {
    let mut methods = Vec::new();
    for entry in walkdir::WalkDir::new(client_src_dir) {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.path().extension().is_some_and(|e| e == "rs") {
            let src = std::fs::read_to_string(entry.path()).map_err(|e| e.to_string())?;
            methods.extend(methods_in_source(&src));
        }
    }
    Ok(methods)
}
