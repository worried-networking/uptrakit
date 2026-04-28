/// Serde helper: serialize/deserialize `UtcDateTime` as Unix epoch milliseconds.
pub mod utc_datetime_millis {
    use serde::{self, Deserialize, Deserializer, Serializer};
    use time::UtcDateTime;
    pub fn serialize<S: Serializer>(dt: &UtcDateTime, serializer: S) -> Result<S::Ok, S::Error> {
        let millis = dt.unix_timestamp_nanos() / 1_000_000;
        let millis_i64 = i64::try_from(millis).map_err(serde::ser::Error::custom)?;
        serializer.serialize_i64(millis_i64)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<UtcDateTime, D::Error> {
        let millis = i64::deserialize(deserializer)?;
        let nanos = i128::from(millis) * 1_000_000;
        UtcDateTime::from_unix_timestamp_nanos(nanos).map_err(serde::de::Error::custom)
    }
}
/// Serde helper: serialize/deserialize `std::time::Duration` as whole seconds (`u32`).
///
/// Uses `u32` consistently across wire, HTTP API, and CLI representations.
/// Maximum representable interval: ~136 years — more than sufficient for ping intervals.
pub mod duration_seconds {
    use serde::{self, Deserialize, Deserializer, Serializer};
    use std::time::Duration;
    pub fn serialize<S: Serializer>(d: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
        let secs = u32::try_from(d.as_secs()).map_err(serde::ser::Error::custom)?;
        serializer.serialize_u32(secs)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
        let secs = u32::deserialize(deserializer)?;
        Ok(Duration::from_secs(u64::from(secs)))
    }
}
/// Serde helper: serialize/deserialize `Option<std::time::Duration>` as optional whole seconds (`u32`).
///
/// Same as [`duration_seconds`] but for optional fields. `None` is omitted from
/// serialization when used with `skip_serializing_if = "Option::is_none"`.
pub mod option_duration_seconds {
    use serde::{self, Deserialize, Deserializer, Serializer};
    use std::time::Duration;
    pub fn serialize<S: Serializer>(
        d: &Option<Duration>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match d {
            Some(d) => {
                let secs = u32::try_from(d.as_secs()).map_err(serde::ser::Error::custom)?;
                serializer.serialize_some(&secs)
            }
            None => serializer.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Duration>, D::Error> {
        let opt = Option::<u32>::deserialize(deserializer)?;
        Ok(opt.map(|secs| Duration::from_secs(u64::from(secs))))
    }
}
