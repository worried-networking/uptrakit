//! CIMD parser for `draft-ietf-oauth-client-id-metadata-document-00`.
//!
//! Extracts only the fields the authorization server needs to validate
//! `redirect_uri` and render the consent screen.  All other fields stay in
//! `metadata_raw` for forward compatibility with future CIMD revisions.

use thiserror::Error;

/// Errors from CIMD document extraction.
///
/// Parse failure is a soft failure at the fetcher layer: per spec §11.3 the
/// previously cached row is preserved so an upstream CIMD draft revision does
/// not trigger a forced re-consent event.
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum CimdParseError {
    /// Required top-level field is absent from the document.
    #[error("missing required field: {0}")]
    MissingField(&'static str),
    /// Required field is present but the JSON type does not match the spec.
    #[error("field has wrong type: {field}")]
    WrongType {
        /// Name of the offending field.
        field: &'static str,
    },
}

/// Parsed fields extracted from a CIMD document.
///
/// Only fields required for `redirect_uri` validation and consent UX are
/// surfaced here. All other fields stay in `metadata_raw` for forward
/// compatibility, per spec §11.3.
///
/// `#[non_exhaustive]`: later CIMD drafts may surface additional consent-UX
/// fields (e.g. `tos_uri`, `policy_uri`, `software_version`).
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CimdDocument {
    /// Stable identifier; must equal the URL the document was fetched from.
    pub client_id: String,
    /// Allowed redirect URIs.  At least one is required for the
    /// authorization-code flow.
    pub redirect_uris: Vec<String>,
    /// Human-readable client name shown on the consent screen.
    pub client_name: String,
}

/// Extract a [`CimdDocument`] from a [`serde_json::Value`].
///
/// Two-pass pattern: the caller already has the [`serde_json::Value`].
/// Extraction failure does not prevent persisting raw bytes — see
/// [`crate::oauth::cimd`] for the surrounding fetcher logic.
///
/// # Errors
/// - [`CimdParseError::MissingField`] if `client_id`, `redirect_uris`, or
///   `client_name` is absent.
/// - [`CimdParseError::WrongType`] if a field is present but the JSON type
///   does not match the spec (`client_id`, `client_name`: string;
///   `redirect_uris`: array of string).
pub fn extract(value: &serde_json::Value) -> Result<CimdDocument, CimdParseError> {
    let client_id = match value.get("client_id") {
        None => return Err(CimdParseError::MissingField("client_id")),
        Some(v) => v
            .as_str()
            .ok_or(CimdParseError::WrongType { field: "client_id" })?
            .to_owned(),
    };

    let redirect_uris = match value.get("redirect_uris") {
        None => return Err(CimdParseError::MissingField("redirect_uris")),
        Some(v) => {
            let arr = v.as_array().ok_or(CimdParseError::WrongType {
                field: "redirect_uris",
            })?;
            let mut out = Vec::with_capacity(arr.len());
            for item in arr {
                let s = item.as_str().ok_or(CimdParseError::WrongType {
                    field: "redirect_uris",
                })?;
                out.push(s.to_owned());
            }
            out
        }
    };

    let client_name = match value.get("client_name") {
        None => return Err(CimdParseError::MissingField("client_name")),
        Some(v) => v
            .as_str()
            .ok_or(CimdParseError::WrongType {
                field: "client_name",
            })?
            .to_owned(),
    };

    Ok(CimdDocument {
        client_id,
        redirect_uris,
        client_name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_extracts_required_fields() {
        let v = serde_json::json!({
            "client_id": "https://client.example.com",
            "redirect_uris": ["https://client.example.com/callback"],
            "client_name": "My App",
        });
        let doc = extract(&v).expect("should parse");
        assert_eq!(doc.client_id, "https://client.example.com");
        assert_eq!(
            doc.redirect_uris,
            vec!["https://client.example.com/callback"]
        );
        assert_eq!(doc.client_name, "My App");
    }

    #[test]
    fn missing_client_id_returns_missing_field() {
        let v = serde_json::json!({
            "redirect_uris": ["https://client.example.com/callback"],
            "client_name": "My App",
        });
        assert!(matches!(
            extract(&v),
            Err(CimdParseError::MissingField("client_id"))
        ));
    }

    #[test]
    fn missing_redirect_uris_returns_missing_field() {
        let v = serde_json::json!({
            "client_id": "https://client.example.com",
            "client_name": "My App",
        });
        assert!(matches!(
            extract(&v),
            Err(CimdParseError::MissingField("redirect_uris"))
        ));
    }

    #[test]
    fn missing_client_name_returns_missing_field() {
        let v = serde_json::json!({
            "client_id": "https://client.example.com",
            "redirect_uris": ["https://client.example.com/callback"],
        });
        assert!(matches!(
            extract(&v),
            Err(CimdParseError::MissingField("client_name"))
        ));
    }

    #[test]
    fn wrong_type_client_id_returns_wrong_type() {
        let v = serde_json::json!({
            "client_id": 42,
            "redirect_uris": ["https://client.example.com/callback"],
            "client_name": "My App",
        });
        assert!(matches!(
            extract(&v),
            Err(CimdParseError::WrongType { field: "client_id" })
        ));
    }

    #[test]
    fn wrong_type_redirect_uris_returns_wrong_type() {
        let v = serde_json::json!({
            "client_id": "https://client.example.com",
            "redirect_uris": "not-an-array",
            "client_name": "My App",
        });
        assert!(matches!(
            extract(&v),
            Err(CimdParseError::WrongType {
                field: "redirect_uris"
            })
        ));
    }

    #[test]
    fn redirect_uris_with_non_string_element_returns_wrong_type() {
        let v = serde_json::json!({
            "client_id": "https://client.example.com",
            "redirect_uris": ["ok", 42],
            "client_name": "My App",
        });
        assert!(matches!(
            extract(&v),
            Err(CimdParseError::WrongType {
                field: "redirect_uris"
            })
        ));
    }

    #[test]
    fn unknown_fields_are_ignored() {
        // Forward compatibility: future CIMD revisions can add fields and
        // the parser must continue to work.
        let v = serde_json::json!({
            "client_id": "https://client.example.com",
            "redirect_uris": ["https://client.example.com/callback"],
            "client_name": "My App",
            "tos_uri": "https://client.example.com/tos",
            "software_version": "1.2.3",
            "future_field_42": { "nested": true },
        });
        let doc = extract(&v).expect("unknown fields should be ignored");
        assert_eq!(doc.client_id, "https://client.example.com");
    }
}
