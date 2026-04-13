use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAX_IDENTIFIER_LEN: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IdentifierError {
    #[error("identifier cannot be empty")]
    Empty,
    #[error("identifier exceeds maximum length of {MAX_IDENTIFIER_LEN} bytes")]
    TooLong,
    #[error("identifier must start with an ASCII lowercase letter")]
    InvalidStart,
    #[error("identifier contains invalid character `{ch}` at byte index {index}")]
    InvalidCharacter { ch: char, index: usize },
}

pub fn validate_surface_identifier(value: &str) -> Result<(), IdentifierError> {
    if value.is_empty() {
        return Err(IdentifierError::Empty);
    }
    if value.len() > MAX_IDENTIFIER_LEN {
        return Err(IdentifierError::TooLong);
    }

    let mut chars = value.char_indices();
    let (_, first) = chars.next().ok_or(IdentifierError::Empty)?;
    if !first.is_ascii_lowercase() {
        return Err(IdentifierError::InvalidStart);
    }

    for (index, ch) in chars {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '_' | '-') {
            continue;
        }
        return Err(IdentifierError::InvalidCharacter { ch, index });
    }

    Ok(())
}

pub fn is_valid_surface_identifier(value: &str) -> bool {
    validate_surface_identifier(value).is_ok()
}

macro_rules! identifier_type {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
                let value = value.into();
                validate_surface_identifier(&value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl std::str::FromStr for $name {
            type Err = IdentifierError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value.to_owned())
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdentifierError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

identifier_type!(SurfaceId);
identifier_type!(InteractionId);
identifier_type!(DataSourceId);
identifier_type!(ControllerQueryId);
identifier_type!(BuiltInApiOperationId);
