//! Conversions between SQLite's signed 64-bit integers and the unsigned values Mellow stores.

use mellow_protocol::Snowflake;

/// A stored or bound integer that does not fit its target type. Each storage error enum wraps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("value does not fit SQLite's signed 64-bit integer")]
pub struct IntegerOutOfRange;

/// SQLite integers are signed; Snowflakes and Unix milliseconds fit until about 2095.
pub(crate) fn to_sqlite_integer(value: u64) -> Result<i64, IntegerOutOfRange> {
    convert_integer(value)
}

/// For lookups: an id SQLite cannot hold cannot exist, so it is simply not found.
pub(crate) fn lookup_integer(id: Snowflake) -> Option<i64> {
    i64::try_from(id.raw()).ok()
}

pub(crate) fn to_snowflake(value: i64) -> Result<Snowflake, IntegerOutOfRange> {
    convert_integer(value).map(Snowflake::from_raw)
}

/// Any checked integer conversion, such as a stored count or position read back as `u32`.
pub(crate) fn convert_integer<Source, Target: TryFrom<Source>>(
    value: Source,
) -> Result<Target, IntegerOutOfRange> {
    Target::try_from(value).map_err(|_| IntegerOutOfRange)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_past_the_signed_range_are_refused() {
        assert_eq!(to_sqlite_integer(u64::MAX), Err(IntegerOutOfRange));
        assert_eq!(lookup_integer(Snowflake::from_raw(u64::MAX)), None);
        assert_eq!(to_snowflake(-1), Err(IntegerOutOfRange));
        assert_eq!(
            convert_integer::<i64, u32>(i64::from(u32::MAX) + 1),
            Err(IntegerOutOfRange)
        );
    }

    #[test]
    fn values_in_range_round_trip() {
        let id = Snowflake::from_raw(42);
        let stored = to_sqlite_integer(id.raw()).expect("42 fits");
        assert_eq!(lookup_integer(id), Some(stored));
        assert_eq!(to_snowflake(stored), Ok(id));
        assert_eq!(convert_integer::<i64, u32>(7), Ok(7_u32));
    }
}
