//! The [`EncryptedString`] type for transparent database encryption.
//!
//! Values are encrypted eagerly at construction time and decrypted on
//! the read path via SeaORM's `TryGetable` implementation.

use std::fmt;

use uptrakit_shared_types::SecretString;

#[cfg(feature = "sea-orm")]
use crate::{ENC_V1_PREFIX, ENC_V2_PREFIX, is_plaintext_mode};
use crate::{ENC_V3_PREFIX, encrypt_str, is_encrypted};

/// A string that is transparently encrypted when written to the database
/// and decrypted when read back.
///
/// Encryption is performed eagerly at construction time via [`EncryptedString::new`].
/// The pre-computed database representation is stored alongside the plaintext
/// so that `From<EncryptedString> for sea_orm::Value` is infallible.
///
/// Construction **requires** the master key (and ideally the data key ring)
/// to be initialized. If neither is available, [`EncryptedString::new`]
/// returns `Err(CryptoError::NotInitialized)`. There is no plaintext
/// fallback -- a missing key is always treated as a hard error to prevent
/// silent secret exposure in misconfigured deployments.
///
/// ## Ciphertext format
///
/// - [`EncryptedString::new`] produces `ENC:v3:` (envelope encryption with
///   DEK + caller-supplied AAD) when the data key ring is initialized, or
///   `ENC:v2:` (KEK-direct with AAD) as fallback.
/// - All three formats (`ENC:v1:`, `ENC:v2:`, `ENC:v3:`) are transparently
///   handled on the read path by the `TryGetable` implementation.
pub struct EncryptedString {
    /// Plaintext value (for `expose_secret`).
    plaintext: SecretString,
    /// Pre-computed value for database storage (encrypted, or plaintext in dev mode).
    pub(crate) db_value: String,
}

impl Clone for EncryptedString {
    fn clone(&self) -> Self {
        Self {
            plaintext: self.plaintext.clone(),
            db_value: self.db_value.clone(),
        }
    }
}

impl PartialEq for EncryptedString {
    fn eq(&self, other: &Self) -> bool {
        // Compare only plaintext -- encrypted values include random nonces
        // so two encryptions of the same value differ.
        self.plaintext.expose_secret() == other.plaintext.expose_secret()
    }
}

impl EncryptedString {
    /// Create a new `EncryptedString` from a plaintext value with
    /// context-bound AAD.
    ///
    /// Produces `ENC:v3:` ciphertext (envelope encryption with DEK) when the
    /// data key ring is initialized, or `ENC:v2:` as fallback (KEK-direct).
    ///
    /// The `aad` string is mixed into the GCM authentication tag, binding
    /// the ciphertext to a specific column/purpose.  Use the
    /// `"uptrakit:<table>:<column>"` convention.
    ///
    /// # Errors
    ///
    /// Returns `Err(CryptoError::NotInitialized)` if the master key has not
    /// been initialized, or `Err` on any other encryption failure.
    pub fn new(plaintext: String, aad: &str) -> crate::Result<Self> {
        let db_value = encrypt_str(&plaintext, aad)?;
        Ok(Self {
            plaintext: SecretString::new(plaintext),
            db_value,
        })
    }

    /// Construct from a decrypted DB value on the read path.
    ///
    /// Used by `ValueType` / `TryGetable` impls to construct from a decrypted
    /// value while preserving the original DB representation.
    #[cfg(feature = "sea-orm")]
    pub(crate) fn from_db(plaintext: String, db_repr: String) -> Self {
        Self {
            plaintext: SecretString::new(plaintext),
            db_value: db_repr,
        }
    }

    /// Expose the plaintext secret.
    pub fn expose_secret(&self) -> &str {
        self.plaintext.expose_secret()
    }

    /// Returns `true` if the stored DB value is not v3 format.
    ///
    /// Used by the re-encryption routine to identify v1/v2/plaintext values
    /// that should be upgraded to `ENC:v3:` format.
    pub fn needs_v3_upgrade(&self) -> bool {
        !self.db_value.starts_with(ENC_V3_PREFIX)
    }

    /// Construct an `EncryptedString` whose database representation is the
    /// raw `value` string **without** the `ENC:v1:` prefix.
    ///
    /// This is used exclusively in tests to simulate legacy rows that were
    /// written to the database before encryption was added, allowing the
    /// re-encryption routine to be tested without raw SQL `UPDATE` statements.
    ///
    /// **Never call this in production code.**
    #[cfg(any(test, feature = "testing"))]
    pub fn plaintext_for_test(value: String) -> Self {
        Self {
            plaintext: SecretString::new(value.clone()),
            db_value: value,
        }
    }

    /// Returns `true` if the stored DB representation is already encrypted.
    ///
    /// Used by the re-encryption routine to identify legacy plaintext values
    /// that need to be re-encrypted with the current master key.
    pub fn is_db_value_encrypted(&self) -> bool {
        is_encrypted(&self.db_value)
    }
}

impl fmt::Debug for EncryptedString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EncryptedString(***)")
    }
}

impl fmt::Display for EncryptedString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***REDACTED***")
    }
}

// -- SeaORM integration --

#[cfg(feature = "sea-orm")]
impl sea_orm::sea_query::ValueType for EncryptedString {
    fn try_from(v: sea_orm::Value) -> std::result::Result<Self, sea_orm::sea_query::ValueTypeErr> {
        match v {
            sea_orm::Value::String(Some(s)) => {
                if s.starts_with(ENC_V3_PREFIX) {
                    // ValueType has no column name -- best-effort with empty AAD.
                    // Normal SeaORM entity queries go through TryGetable which has
                    // the column name and can look up the correct AAD.
                    let plaintext =
                        crate::decrypt_str(&s, "").map_err(|_| sea_orm::sea_query::ValueTypeErr)?;
                    Ok(Self::from_db(plaintext, s))
                } else if s.starts_with(ENC_V1_PREFIX) {
                    let plaintext =
                        crate::decrypt_str(&s, "").map_err(|_| sea_orm::sea_query::ValueTypeErr)?;
                    Ok(Self::from_db(plaintext, s))
                } else if s.starts_with(ENC_V2_PREFIX) {
                    // ValueType has no column name -- use empty AAD as fallback.
                    let plaintext =
                        crate::decrypt_str(&s, "").map_err(|_| sea_orm::sea_query::ValueTypeErr)?;
                    Ok(Self::from_db(plaintext, s))
                } else if is_plaintext_mode() {
                    // Plaintext mode -- accept as-is
                    Ok(Self::from_db(s.clone(), s))
                } else {
                    // Legacy plaintext -- accept as-is
                    Ok(Self::from_db(s.clone(), s))
                }
            }
            _ => Err(sea_orm::sea_query::ValueTypeErr),
        }
    }

    fn type_name() -> String {
        "EncryptedString".to_string()
    }

    fn array_type() -> sea_orm::sea_query::ArrayType {
        sea_orm::sea_query::ArrayType::String
    }

    fn column_type() -> sea_orm::sea_query::ColumnType {
        sea_orm::sea_query::ColumnType::Text
    }
}

#[cfg(feature = "sea-orm")]
impl sea_orm::sea_query::Nullable for EncryptedString {
    fn null() -> sea_orm::Value {
        sea_orm::Value::String(None)
    }
}

#[cfg(feature = "sea-orm")]
impl From<EncryptedString> for sea_orm::Value {
    fn from(val: EncryptedString) -> Self {
        sea_orm::Value::String(Some(val.db_value))
    }
}

#[cfg(feature = "sea-orm")]
impl sea_orm::TryGetable for EncryptedString {
    fn try_get_by<I: sea_orm::ColIdx>(
        res: &sea_orm::QueryResult,
        index: I,
    ) -> std::result::Result<Self, sea_orm::TryGetError> {
        let s: Option<String> = res.try_get_by(index).map_err(sea_orm::TryGetError::DbErr)?;
        let s = match s {
            Some(s) => s,
            None => {
                let column_name = match index.as_str() {
                    Some(name) => name.to_string(),
                    None => "encrypted_string".to_string(),
                };
                return Err(sea_orm::TryGetError::Null(column_name));
            }
        };

        if s.starts_with(ENC_V3_PREFIX) {
            let col_name = index.as_str().unwrap_or("unknown");
            let aad = crate::column_aad(col_name).unwrap_or("");
            let plaintext = crate::decrypt_str(&s, aad).map_err(|e| {
                sea_orm::TryGetError::DbErr(sea_orm::DbErr::Type(format!(
                    "ENC:v3 decryption failed for column '{col_name}': {e}"
                )))
            })?;
            Ok(Self::from_db(plaintext, s))
        } else if s.starts_with(ENC_V2_PREFIX) {
            let col_name = index.as_str().unwrap_or("unknown");
            let aad = crate::column_aad(col_name).unwrap_or("");
            let plaintext = crate::decrypt_str(&s, aad).map_err(|e| {
                sea_orm::TryGetError::DbErr(sea_orm::DbErr::Type(format!(
                    "ENC:v2 decryption failed for column '{col_name}': {e}"
                )))
            })?;
            Ok(Self::from_db(plaintext, s))
        } else if s.starts_with(ENC_V1_PREFIX) {
            let plaintext = crate::decrypt_str(&s, "").map_err(|e| {
                sea_orm::TryGetError::DbErr(sea_orm::DbErr::Type(format!(
                    "EncryptedString decryption failed: {e}"
                )))
            })?;
            Ok(Self::from_db(plaintext, s))
        } else {
            // Legacy plaintext -- accept as-is
            Ok(Self::from_db(s.clone(), s))
        }
    }
}
