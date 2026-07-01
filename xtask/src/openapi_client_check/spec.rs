//! OpenAPI spec parsing: flatten the document into one `SpecOp` per operation.

use serde_json::Value;

/// One OpenAPI operation carrying an `operationId`.
#[derive(Debug, Clone)]
pub struct SpecOp {
    pub operation_id: String,
    /// Placeholder-normalized path template (see `super::normalize::normalize_path`).
    pub path: String,
    pub method: String,
}

const METHODS: &[&str] = &["get", "put", "post", "delete", "patch", "head", "options"];

/// Parse an OpenAPI JSON document into its operations.
///
/// # Errors
/// Returns an error string if the JSON is malformed or `paths` is missing/not an object.
pub fn load(json: &str) -> Result<Vec<SpecOp>, String> {
    let doc: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let Some(paths) = doc.get("paths").and_then(Value::as_object) else {
        return Err("openapi document has no `paths` object".to_string());
    };
    let mut ops = Vec::new();
    for (path, item) in paths {
        let Some(item_obj) = item.as_object() else {
            continue;
        };
        for method in METHODS {
            let Some(op) = item_obj.get(*method).and_then(Value::as_object) else {
                continue;
            };
            if let Some(id) = op.get("operationId").and_then(Value::as_str) {
                ops.push(SpecOp {
                    operation_id: id.to_string(),
                    path: super::normalize::normalize_path(path),
                    method: (*method).to_string(),
                });
            }
        }
    }
    Ok(ops)
}
