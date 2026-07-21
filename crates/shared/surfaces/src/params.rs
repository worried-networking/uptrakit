use serde::{Deserialize, Serialize};

use crate::SchemaContract;

/// Framework-reserved / envelope query keys. Provider-declared `params` keys
/// must not collide with these (admission rule, spec §4 rule 1). `id` is the
/// B5 item-addressing key populated from the `/{item_id}` path segment.
pub const RESERVED_PARAM_KEYS: &[&str] = &[
    "page",
    "per_page",
    "target_provider_id",
    "timeout_seconds",
    "id",
];

/// Opt-in per-field parameter declaration on an interaction. Declared fields
/// get strict typed parsing on GET query strings and per-field body
/// validation on mutating methods; undeclared keys pass through untyped.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParamFieldDescriptor {
    pub key: String,
    pub schema: SchemaContract,
    #[serde(default)]
    pub required: bool,
}

impl ParamFieldDescriptor {
    pub fn new(key: impl Into<String>, schema: SchemaContract) -> Self {
        Self {
            key: key.into(),
            schema,
            required: false,
        }
    }

    #[must_use]
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_field_descriptor_defaults_required_false_on_wire() {
        let json = serde_json::json!({ "key": "plugin_config_id", "schema": { "type": "string" } });
        let field: ParamFieldDescriptor = serde_json::from_value(json).expect("deserialize");
        assert!(!field.required);
        assert_eq!(field.schema, crate::SchemaContract::String);
    }

    #[test]
    fn reserved_keys_cover_the_b5_id_key() {
        assert!(RESERVED_PARAM_KEYS.contains(&"id"));
        assert!(RESERVED_PARAM_KEYS.contains(&"page"));
        assert!(RESERVED_PARAM_KEYS.contains(&"per_page"));
        assert!(RESERVED_PARAM_KEYS.contains(&"target_provider_id"));
        assert!(RESERVED_PARAM_KEYS.contains(&"timeout_seconds"));
    }
}
