/// Generate three `#[ignore]` test functions — one per DB backend.
///
/// Each generated test is `#[tokio::test]` + `#[ignore]` and constructs
/// a [`TestHarness`](super::harness::TestHarness) for the corresponding
/// backend before running the async test function.
///
/// The test function must have the signature:
/// `async fn(&TestHarness) -> ()`
///
/// # Usage
///
/// ```rust,ignore
/// async fn test_something(harness: &TestHarness) {
///     let client = harness.client();
///     // ...
/// }
/// db_test!(something, test_something);
/// ```
macro_rules! db_test {
    ($name:ident, $test_fn:path) => {
        paste::paste! {
            #[tokio::test]
            #[ignore = "Database integration test (requires Docker for PG/MariaDB)"]
            async fn [<$name _sqlite>]() {
                $crate::database_helpers::harness::init_test_tracing();
                let harness = $crate::database_helpers::harness::TestHarness::new_sqlite().await;
                $test_fn(&harness).await;
            }

            #[tokio::test]
            #[ignore = "Database integration test (requires Docker for PG/MariaDB)"]
            async fn [<$name _postgres>]() {
                $crate::database_helpers::harness::init_test_tracing();
                let harness = $crate::database_helpers::harness::TestHarness::new_postgres().await;
                $test_fn(&harness).await;
            }

            #[tokio::test]
            #[ignore = "Database integration test (requires Docker for PG/MariaDB)"]
            async fn [<$name _mariadb>]() {
                $crate::database_helpers::harness::init_test_tracing();
                let harness = $crate::database_helpers::harness::TestHarness::new_mariadb().await;
                $test_fn(&harness).await;
            }
        }
    };
}
pub(crate) use db_test;
