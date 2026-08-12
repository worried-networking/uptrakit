//! [`encrypted_column!`] — generates a SeaORM column newtype over
//! [`EncryptedString`](crate::EncryptedString) with a compile-time AAD.
//!
//! Gated on the `sea-orm` feature at the module level (not per trait impl
//! inside the macro): a `#[cfg(feature = "sea-orm")]` written *inside* a
//! `macro_rules!` expansion would evaluate against the *consuming* crate's
//! feature set, not this crate's — a cross-crate cfg leak. Every consumer of
//! this macro already enables `uptrakit-crypto/sea-orm` unconditionally, so
//! gating the whole module here costs nothing.

/// Generate a SeaORM column newtype over [`EncryptedString`](crate::EncryptedString) with a
/// compile-time AAD: `encrypted_column!(TypeName, "uptrakit:<table>:<column>");`
///
/// The generated type eagerly parses the plaintext as JSON at construction
/// and at decode, so `as_json()`/`expose_secret()` are infallible accessors.
#[macro_export]
macro_rules! encrypted_column {
    ($(#[$meta:meta])* $name:ident, $aad:literal) => {
        $(#[$meta])*
        pub struct $name {
            inner: $crate::EncryptedString,
            json: $crate::serde_json::Value,
        }

        impl $name {
            /// Compile-time AAD bound into every encrypt/decrypt for this column.
            pub const AAD: &'static str = $aad;

            /// Encrypt a JSON document given as a string; the string must parse.
            ///
            /// # Errors
            ///
            /// `CryptoError::InvalidJson` when the string is not valid JSON;
            /// otherwise any encryption error (e.g. `NotInitialized`).
            pub fn new(plaintext_json: String) -> $crate::Result<Self> {
                let json: $crate::serde_json::Value = $crate::serde_json::from_str(&plaintext_json)
                    .map_err(|e| $crate::rootcause::report!($crate::CryptoError::InvalidJson(e.to_string())))?;
                let inner = $crate::EncryptedString::new(plaintext_json, Self::AAD)?;
                Ok(Self { inner, json })
            }

            /// Encrypt an in-memory JSON value.
            ///
            /// # Errors
            ///
            /// Any encryption error (e.g. `NotInitialized`).
            pub fn from_json(value: &$crate::serde_json::Value) -> $crate::Result<Self> {
                let inner = $crate::EncryptedString::new(value.to_string(), Self::AAD)?;
                Ok(Self { inner, json: value.clone() })
            }

            fn decode(db_repr: String) -> $crate::Result<Self> {
                let inner = $crate::EncryptedString::decode_db_value(db_repr, Self::AAD)?;
                let json: $crate::serde_json::Value = $crate::serde_json::from_str(inner.expose_secret())
                    .map_err(|e| $crate::rootcause::report!($crate::CryptoError::InvalidJson(e.to_string())))?;
                Ok(Self { inner, json })
            }

            /// The decrypted config as JSON (infallible: parsed at construction).
            pub fn as_json(&self) -> &$crate::serde_json::Value {
                &self.json
            }

            /// The decrypted config as its raw JSON string.
            pub fn expose_secret(&self) -> &str {
                self.inner.expose_secret()
            }

            /// True when the stored DB value is not `ENC:v3:` yet.
            pub fn needs_v3_upgrade(&self) -> bool {
                self.inner.needs_v3_upgrade()
            }

            /// True when the stored DB representation is encrypted at all.
            pub fn is_db_value_encrypted(&self) -> bool {
                self.inner.is_db_value_encrypted()
            }
        }


        impl Clone for $name {
            fn clone(&self) -> Self {
                Self {
                    inner: self.inner.clone(),
                    json: self.json.clone(),
                }
            }
        }

        impl PartialEq for $name {
            fn eq(&self, other: &Self) -> bool {
                // Structural comparison of the parsed JSON — ciphertexts
                // contain random nonces, and Postgres jsonb→text
                // normalization must not create phantom diffs.
                self.json == other.json
            }
        }

        impl PartialEq<$crate::serde_json::Value> for $name {
            fn eq(&self, other: &$crate::serde_json::Value) -> bool {
                &self.json == other
            }
        }

        impl ::std::fmt::Debug for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(concat!(stringify!($name), "(***)"))
            }
        }

        impl sea_orm::sea_query::ValueType for $name {
            #[expect(
                clippy::map_err_ignore,
                reason = "ValueTypeErr is a unit struct that carries no additional context; the original error is discarded intentionally"
            )]
            fn try_from(v: sea_orm::Value) -> Result<Self, sea_orm::sea_query::ValueTypeErr> {
                match v {
                    sea_orm::Value::String(Some(s)) => {
                        Self::decode(s).map_err(|_| sea_orm::sea_query::ValueTypeErr)
                    }
                    _ => Err(sea_orm::sea_query::ValueTypeErr),
                }
            }

            fn type_name() -> String {
                stringify!($name).to_string()
            }

            fn array_type() -> sea_orm::sea_query::ArrayType {
                sea_orm::sea_query::ArrayType::String
            }

            fn column_type() -> sea_orm::sea_query::ColumnType {
                sea_orm::sea_query::ColumnType::Text
            }
        }

        impl sea_orm::sea_query::Nullable for $name {
            fn null() -> sea_orm::Value {
                sea_orm::Value::String(None)
            }
        }

        impl From<$name> for sea_orm::Value {
            fn from(val: $name) -> Self {
                val.inner.into()
            }
        }

        impl sea_orm::TryGetable for $name {
            fn try_get_by<I: sea_orm::ColIdx>(
                res: &sea_orm::QueryResult,
                index: I,
            ) -> Result<Self, sea_orm::TryGetError> {
                let s: Option<String> = res.try_get_by(index).map_err(sea_orm::TryGetError::DbErr)?;
                let Some(s) = s else {
                    let column_name = index
                        .as_str()
                        .map_or_else(|| "encrypted_column".to_string(), ToString::to_string);
                    return Err(sea_orm::TryGetError::Null(column_name));
                };
                // Row identity for operators: TryGetable cannot see the primary key,
                // so the ciphertext prefix (version tag + key-id + per-row random
                // nonce — never plaintext) is the only greppable handle
                // (`WHERE config LIKE 'ENC:v3:<key_id>:<nonce>%'`). 40 chars reaches
                // through the 24-hex nonce; 16 would stop at the key-id, identical
                // for every row under the active DEK. A plaintext row failing JSON
                // parse contributes only its length.
                let handle = if s.starts_with("ENC:") {
                    s.chars().take(40).collect::<String>()
                } else {
                    format!("<plaintext, {} bytes>", s.len())
                };
                Self::decode(s).map_err(|e| {
                    sea_orm::TryGetError::DbErr(sea_orm::DbErr::Type(format!(
                        "encrypted column decode failed ({}, row value {handle}): {e}",
                        Self::AAD
                    )))
                })
            }
        }
    };
}

// No local (same-crate) `encrypted_column!` instantiation here for tests — a
// documented deviation from the brief's literal wording. rustc/clippy's
// `#[expect]` fulfillment tracking does not register the lint firing on
// `ValueType::try_from` when the exported macro is expanded *within its own
// defining crate*: an in-crate `encrypted_column!(TestColumn, ...)` inside a
// `#[cfg(test)] mod tests` here (and even a non-test top-level instantiation)
// both reproduce `error: this lint expectation is unfulfilled` on the single
// `#[expect(clippy::map_err_ignore, ...)]` this module carries, even though
// the generated code is identical to the real `uptrakit-shared-db`
// instantiations, which clippy-check clean. The macro is exercised instead
// from `tests/encrypted_column.rs`, a true cross-crate invocation — the same
// situation as every real consumer — where the `#[expect]` is correctly
// tracked as fulfilled.
