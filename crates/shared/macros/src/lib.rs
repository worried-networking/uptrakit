//! This crate provides foundational macros for the project's error handling
//! and wire protocol patterns.
//!
//! # Macros
//!
//! - [`impl_report_conversion!`] — reduces boilerplate for converting a
//!   `Report<SourceError>` into a `Report<TargetError>` using `context_transform`.
//! - [`wire_safe_enum!`] — generates a forward-compatible wire enum with an
//!   `Other(String)` catch-all variant, `as_str`, `Display`, `From<String>`,
//!   `Serialize`, `Deserialize`, and a strict `FromStr`.
//!
//! For a comprehensive understanding of the error handling strategy, including
//! the role of `rootcause` and `thiserror`, please refer to the
//! [Coding Standards documentation](../../docs/development/coding-standards.md).

/// Generate `ReportConversion` implementations to enable `.context_to()` conversions.
///
/// Reduces boilerplate for the common pattern of converting a `Report<SourceError>`
/// into a `Report<TargetError>` using `context_transform`.
///
/// # Simple variant mapping
///
/// When the source error type maps directly to a variant of the target error
/// (i.e. the variant wraps the source type via `#[from]`):
///
/// ```ignore
/// impl_report_conversion!(sea_orm::DbErr => MyError::Database);
/// ```
///
/// Expands to:
///
/// ```ignore
/// impl<T> ReportConversion<sea_orm::DbErr, markers::Mutable, T> for MyError
/// where
///     MyError: markers::ObjectMarkerFor<T>,
/// {
///     fn convert_report(
///         report: Report<sea_orm::DbErr, markers::Mutable, T>,
///     ) -> Report<Self, markers::Mutable, T> {
///         report.context_transform(MyError::Database)
///     }
/// }
/// ```
///
/// ## Target identifier constraint
///
/// The `$target` fragment accepts a **single identifier** (e.g. `MyError`), not a
/// multi-segment path (e.g. `module::MyError`). This is intentional and acceptable
/// because `impl_report_conversion!` is always invoked in the same module where the
/// target error type is defined, so the single-identifier form is always sufficient.
///
/// # Closure-based transform
///
/// When extra wrapping is needed (e.g. `Box::new`, `to_string`, or pattern matching):
///
/// ```ignore
/// impl_report_conversion!(tungstenite::Error => ControllerError, |e| ControllerError::WebSocket(Box::new(e)));
/// ```
///
/// Expands to:
///
/// ```ignore
/// impl<T> ReportConversion<tungstenite::Error, markers::Mutable, T> for ControllerError
/// where
///     ControllerError: markers::ObjectMarkerFor<T>,
/// {
///     fn convert_report(
///         report: Report<tungstenite::Error, markers::Mutable, T>,
///     ) -> Report<Self, markers::Mutable, T> {
///         report.context_transform(|e| ControllerError::WebSocket(Box::new(e)))
///     }
/// }
/// ```
///
/// # Multiple conversions
///
/// **Note:** Closure-based transforms must be written as separate single
/// invocations and cannot be mixed in the multi-conversion syntax.
///
/// ```ignore
/// impl_report_conversion! {
///     sea_orm::DbErr => MyError::Database,
///     std::io::Error => MyError::Io,
///     serde_json::Error => MyError::Json,
/// }
/// ```
///
/// # Dependency requirement
///
/// This macro hard-codes the crate name `rootcause`. The macro will fail to
/// expand if the downstream crate has renamed the dependency in its `Cargo.toml`
/// (e.g. `rootcause = { package = "rootcause", rename = "rc" }`). All workspace
/// crates must keep `rootcause` under its canonical name.
#[macro_export]
macro_rules! impl_report_conversion {
    // Single: simple variant mapping
    ($source:ty => $target:ident :: $variant:ident) => {
        impl<T> rootcause::ReportConversion<$source, rootcause::prelude::markers::Mutable, T>
            for $target
        where
            $target: rootcause::prelude::markers::ObjectMarkerFor<T>,
        {
            fn convert_report(
                report: rootcause::prelude::Report<
                    $source,
                    rootcause::prelude::markers::Mutable,
                    T,
                >,
            ) -> rootcause::prelude::Report<
                Self,
                rootcause::prelude::markers::Mutable,
                T,
            > {
                report.context_transform($target::$variant)
            }
        }
    };

    // Single: closure-based transform
    ($source:ty => $target:ident, $closure:expr) => {
        impl<T> rootcause::ReportConversion<$source, rootcause::prelude::markers::Mutable, T>
            for $target
        where
            $target: rootcause::prelude::markers::ObjectMarkerFor<T>,
        {
            fn convert_report(
                report: rootcause::prelude::Report<
                    $source,
                    rootcause::prelude::markers::Mutable,
                    T,
                >,
            ) -> rootcause::prelude::Report<
                Self,
                rootcause::prelude::markers::Mutable,
                T,
            > {
                report.context_transform($closure)
            }
        }
    };

    // Multiple: trailing comma support
    ($($source:ty => $target:ident :: $variant:ident),+ $(,)?) => {
        $(
            $crate::impl_report_conversion!($source => $target::$variant);
        )+
    };
}

/// Generate a forward-compatible wire enum with an `Other(String)` catch-all.
///
/// # Syntax
///
/// ```ignore
/// wire_safe_enum! {
///     /// Doc comment for the enum.
///     #[derive(Clone, Debug, PartialEq, Eq, Hash)]
///     pub enum MyEnum {
///         VariantOne => "variant_one",
///         VariantTwo => "variant_two",
///     }
///     parse_error = ParseMyEnumError("invalid my enum");
/// }
/// ```
///
/// Do **not** add `#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]`:
/// the derive documents Rust variant identifiers (PascalCase), which do not match
/// the serde wire strings. The macro instead emits a manual `utoipa::ToSchema`
/// (gated on the invoking crate's `openapi` feature) that documents the
/// `=> "wire"` strings — see generated item 9 below.
///
/// The `parse_error` line specifies the name of the strict-parse error type and the
/// human-readable error message used in its [`thiserror::Error`] impl.
///
/// # Generated items
///
/// 1. The enum definition with `#[non_exhaustive]` and an `Other(String)` variant
///    appended automatically. Do **not** include `Other` in the input body.
/// 2. `impl MyEnum { pub fn as_str(&self) -> &str { ... } }` — known variants return
///    `&'static str`; `Other(s)` returns `s.as_str()`.
/// 3. `impl std::fmt::Display for MyEnum` — delegates to `as_str`.
/// 4. `impl From<String> for MyEnum` — unknown strings map to `Other(s)` and emit
///    `tracing::debug!`.
/// 5. `impl serde::Serialize for MyEnum` — serializes via `as_str`.
/// 6. `impl<'de> serde::Deserialize<'de> for MyEnum` — infallible; unknown strings
///    map to `Other(s)`.
/// 7. The `ParseMyEnumError` struct (name taken from `parse_error =`) with
///    `#[derive(Debug, thiserror::Error)]` and the provided `#[error(...)]` message.
/// 8. `impl std::str::FromStr for MyEnum` — strict; returns `Err(ParseMyEnumError)`
///    for unknown strings.
/// 9. `#[cfg(feature = "openapi")] impl utoipa::PartialSchema + ToSchema for MyEnum`
///    — emits a `{ "type": "string", "enum": [...wire strings...] }` schema (the
///    `=> "..."` RHS values, excluding `Other`). Only compiled when the invoking
///    crate enables an `openapi` feature; inert (and `utoipa`-free) otherwise.
/// 10. `#[cfg(feature = "schema")] impl schemars::JsonSchema for MyEnum` — emits
///     an OPEN string schema `{ "type": "string", "description": "... Known values:
///     ..." }` with no `"enum"` array (the `Other` catch-all makes the value space
///     open). Wire strings are derived from the macro's `=> "..."` inputs, not
///     hardcoded. Only compiled when the invoking crate enables a `schema` feature.
///
/// # Constraints
///
/// - The downstream crate must have `serde`, `thiserror`, and `tracing` as dependencies.
/// - `#[non_exhaustive]` is always emitted by the macro — do not add it in the input.
/// - The `Other(String)` catch-all is always appended by the macro — never include it
///   in the input body.
/// - `Copy` is not possible because `Other(String)` contains a `String`.
/// - Individual variants may carry their own attributes (e.g. `#[default]` paired
///   with `Default` in the enum-level `#[derive(...)]`) by placing them before the
///   variant name: `#[default]\nPost => "post",`.
#[macro_export]
macro_rules! wire_safe_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $($(#[$vmeta:meta])* $variant:ident => $wire:literal),+ $(,)?
        }
        parse_error = $err_name:ident($err_msg:literal);
    ) => {
        $(#[$meta])*
        #[non_exhaustive]
        $vis enum $name {
            $($(#[$vmeta])* $variant,)+
            /// An unknown value received from a newer peer.
            ///
            /// The inner string is the raw value as it appeared on the wire.
            Other(::std::string::String),
        }

        impl $name {
            /// Returns the wire-format string for this value.
            ///
            /// For the `Other` variant, returns the inner string as-is.
            pub fn as_str(&self) -> &str {
                match self {
                    $(Self::$variant => $wire,)+
                    Self::Other(s) => s.as_str(),
                }
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl ::std::convert::From<::std::string::String> for $name {
            fn from(s: ::std::string::String) -> Self {
                match s.as_str() {
                    $($wire => Self::$variant,)+
                    _ => {
                        ::tracing::debug!(
                            value = s,
                            "received unknown {} value",
                            ::std::stringify!($name),
                        );
                        Self::Other(s)
                    }
                }
            }
        }

        impl ::serde::Serialize for $name {
            fn serialize<S: ::serde::Serializer>(
                &self,
                serializer: S,
            ) -> ::std::result::Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(
                deserializer: D,
            ) -> ::std::result::Result<Self, D::Error> {
                ::std::string::String::deserialize(deserializer).map($name::from)
            }
        }

        /// Error returned by the strict [`std::str::FromStr`] impl.
        ///
        /// This error is only returned by `FromStr` (strict user-input parsing).
        /// Serde deserialization is infallible and maps unknown strings to the
        /// `Other` variant instead.
        #[derive(Debug, ::thiserror::Error)]
        #[error($err_msg)]
        $vis struct $err_name;

        impl ::std::str::FromStr for $name {
            type Err = $err_name;

            fn from_str(s: &str) -> ::std::result::Result<Self, Self::Err> {
                match s {
                    $($wire => ::std::result::Result::Ok(Self::$variant),)+
                    _ => ::std::result::Result::Err($err_name),
                }
            }
        }

        // OpenAPI schema describing the *serde wire format* of this enum.
        //
        // The derived `utoipa::ToSchema` would document the Rust variant
        // identifiers (PascalCase) because it is blind to the custom infallible
        // serde impls above (which map each variant to its `=> "wire"` string).
        // This manual impl emits the snake_case/lowercase wire strings instead,
        // so the generated client matches the live API byte-for-byte.
        //
        // The `Other` catch-all is intentionally excluded: it is a decode-only
        // forward-compatibility mechanism, not a documented, producible value.
        //
        // `cfg(feature = "openapi")` is evaluated in the *invoking* crate; crates
        // without that feature (or without a `utoipa` dependency) simply omit it.
        #[cfg(feature = "openapi")]
        impl ::utoipa::PartialSchema for $name {
            fn schema() -> ::utoipa::openapi::RefOr<::utoipa::openapi::schema::Schema> {
                ::utoipa::openapi::ObjectBuilder::new()
                    .schema_type(::utoipa::openapi::schema::Type::String)
                    .enum_values(::std::option::Option::Some([$($wire),+]))
                    .into()
            }
        }

        #[cfg(feature = "openapi")]
        impl ::utoipa::ToSchema for $name {
            fn name() -> ::std::borrow::Cow<'static, str> {
                ::std::borrow::Cow::Borrowed(::std::stringify!($name))
            }
        }

        // JSON Schema (schemars) describing the *serde wire format* — the
        // schemars mirror of the utoipa impl above. Open string schema: the
        // Other catch-all makes the value space open, so no closed enum list.
        // `cfg(feature = "schema")` is evaluated in the *invoking* crate.
        #[cfg(feature = "schema")]
        impl ::schemars::JsonSchema for $name {
            fn schema_name() -> ::std::borrow::Cow<'static, str> {
                ::std::borrow::Cow::Borrowed(::std::stringify!($name))
            }

            fn json_schema(_: &mut ::schemars::SchemaGenerator) -> ::schemars::Schema {
                ::schemars::json_schema!({
                    "type": "string",
                    "description": ::std::format!(
                        "Open wire string (unknown values are forward-compatible). Known values: {}.",
                        [$($wire),+].join(", ")
                    ),
                })
            }
        }
    };
}
