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
/// # Closure-based transform
///
/// When extra wrapping is needed (e.g. `Box::new`, `to_string`, or pattern matching):
///
/// ```ignore
/// impl_report_conversion!(tungstenite::Error => ControllerError, |e| ControllerError::WebSocket(Box::new(e)));
/// ```
///
/// # Multiple conversions
///
/// ```ignore
/// impl_report_conversion! {
///     sea_orm::DbErr => MyError::Database,
///     std::io::Error => MyError::Io,
///     serde_json::Error => MyError::Json,
/// }
/// ```
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
