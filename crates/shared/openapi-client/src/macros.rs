#[allow(unused_macros)]
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
            impl_report_conversion!($source => $target::$variant);
        )+
    };
}

#[allow(unused_macros)]
macro_rules! wire_safe_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $($variant:ident => $wire:literal),+ $(,)?
        }
        parse_error = $err_name:ident($err_msg:literal);
    ) => {
        $(#[$meta])*
        #[non_exhaustive]
        $vis enum $name {
            $($variant,)+
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
    };
}
