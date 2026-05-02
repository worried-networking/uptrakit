// All helpers in this module are consumed only by sibling modules (`notifications.rs`,
// `proxmox_add_config.rs`) which are themselves pending wiring via `local_executor.rs`.
// Until `local_executor.rs` is incorporated, these helpers have no compiled entry point,
// triggering dead_code. Remove this allow once `local_executor.rs` is wired in.
#![allow(
    dead_code,
    reason = "all helpers are consumed by sibling modules pending wiring via local_executor.rs"
)]

use uuid::Uuid;

pub(super) fn required_string_param(
    params: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<String, String> {
    let Some(value) = params.get(key) else {
        return Err(format!("missing required field `{key}`"));
    };
    let Some(value) = value.as_str() else {
        return Err(format!("field `{key}` must be a string"));
    };
    if value.trim().is_empty() {
        return Err(format!("field `{key}` must not be empty"));
    }
    Ok(value.to_string())
}

pub(super) fn optional_string_param(
    params: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Option<String>, String> {
    let Some(value) = params.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let Some(value) = value.as_str() else {
        return Err(format!("field `{key}` must be a string"));
    };
    Ok(Some(value.to_string()))
}

pub(super) fn required_uuid_param(
    params: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Uuid, String> {
    let value = required_string_param(params, key)?;
    Uuid::parse_str(value.as_str())
        .map_err(|error| format!("field `{key}` must be a UUID: {error}"))
}

pub(super) fn strict_bool_param_with_default(
    params: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    default: bool,
) -> Result<bool, String> {
    let Some(value) = params.get(key) else {
        return Ok(default);
    };
    let Some(value) = value.as_bool() else {
        return Err(format!("field `{key}` must be a boolean"));
    };
    Ok(value)
}

pub(super) fn strict_optional_bool_param(
    params: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Option<bool>, String> {
    let Some(value) = params.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let Some(value) = value.as_bool() else {
        return Err(format!("field `{key}` must be a boolean"));
    };
    Ok(Some(value))
}

pub(super) fn parse_csv_array_or_string_array_param(
    params: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Vec<String>, String> {
    let Some(value) = params.get(key) else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }

    match value {
        serde_json::Value::String(text) => Ok(text
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(str::to_string)
            .collect()),
        serde_json::Value::Array(values) => {
            let mut parsed = Vec::new();
            for value in values {
                let Some(value) = value.as_str() else {
                    return Err(format!("field `{key}` array entries must be strings"));
                };
                let value = value.trim();
                if !value.is_empty() {
                    parsed.push(value.to_string());
                }
            }
            Ok(parsed)
        }
        _ => Err(format!(
            "field `{key}` must be either a csv string or an array of strings"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        optional_string_param, parse_csv_array_or_string_array_param, required_string_param,
        required_uuid_param, strict_bool_param_with_default, strict_optional_bool_param,
    };

    #[test]
    fn required_string_param_rejects_missing_or_blank_values() {
        let missing = serde_json::json!({});
        let missing = missing.as_object().expect("object");
        assert_eq!(
            required_string_param(missing, "name").expect_err("missing must fail"),
            "missing required field `name`"
        );

        let blank = serde_json::json!({ "name": "   " });
        let blank = blank.as_object().expect("object");
        assert_eq!(
            required_string_param(blank, "name").expect_err("blank must fail"),
            "field `name` must not be empty"
        );
    }

    #[test]
    fn optional_string_param_treats_missing_and_null_as_none() {
        let missing = serde_json::json!({});
        let missing = missing.as_object().expect("object");
        assert_eq!(
            optional_string_param(missing, "token").expect("missing is allowed"),
            None
        );

        let null_value = serde_json::json!({ "token": null });
        let null_value = null_value.as_object().expect("object");
        assert_eq!(
            optional_string_param(null_value, "token").expect("null is allowed"),
            None
        );
    }

    #[test]
    fn required_uuid_param_rejects_invalid_uuid() {
        let params = serde_json::json!({ "id": "not-a-uuid" });
        let params = params.as_object().expect("object");
        let err = required_uuid_param(params, "id").expect_err("invalid uuid should fail");
        assert!(
            err.contains("field `id` must be a UUID"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn strict_bool_helpers_reject_non_boolean_values() {
        let strict = serde_json::json!({ "enabled": 1 });
        let strict = strict.as_object().expect("object");
        assert_eq!(
            strict_bool_param_with_default(strict, "enabled", true)
                .expect_err("non-boolean should fail"),
            "field `enabled` must be a boolean"
        );

        let optional = serde_json::json!({ "enabled": "false" });
        let optional = optional.as_object().expect("object");
        assert_eq!(
            strict_optional_bool_param(optional, "enabled").expect_err("string should fail"),
            "field `enabled` must be a boolean"
        );
    }

    #[test]
    fn parse_csv_array_or_string_array_param_normalizes_csv_and_arrays() {
        let csv = serde_json::json!({ "node_filter": " node-a,, node-b " });
        let csv = csv.as_object().expect("object");
        assert_eq!(
            parse_csv_array_or_string_array_param(csv, "node_filter").expect("csv should parse"),
            vec!["node-a".to_string(), "node-b".to_string()]
        );

        let array = serde_json::json!({ "node_filter": [" node-a ", "", "node-b"] });
        let array = array.as_object().expect("object");
        assert_eq!(
            parse_csv_array_or_string_array_param(array, "node_filter")
                .expect("array should parse"),
            vec!["node-a".to_string(), "node-b".to_string()]
        );
    }
}
