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
