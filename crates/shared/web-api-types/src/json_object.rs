/// Private, crate-internal building block shared by the public newtypes that need to enforce
/// "JSON object only" with a call-site-specific `ValidationError.field` name.
pub(crate) fn parse_json_object(
    value: serde_json::Value,
    field: &'static str,
) -> Result<serde_json::Map<String, serde_json::Value>, crate::validation::ValidationError> {
    match value {
        serde_json::Value::Object(map) => Ok(map),
        _ => Err(crate::validation::ValidationError {
            field,
            message: "must be a JSON object".to_string(),
        }),
    }
}

pub(crate) fn validate_json_object(
    value: &serde_json::Value,
    field: &'static str,
) -> Result<(), crate::validation::ValidationError> {
    if value.is_object() {
        Ok(())
    } else {
        Err(crate::validation::ValidationError {
            field,
            message: "must be a JSON object".to_string(),
        })
    }
}
