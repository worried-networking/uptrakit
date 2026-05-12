/// In-process delta carrying the new value for one Config Section.
///
/// Wire-incompatible by design: never serialised.
/// Full implementation added in Task 8.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum RuntimeConfigDelta {
    /// Placeholder variant — concrete section variants added in Task 8.
    _Placeholder,
}
