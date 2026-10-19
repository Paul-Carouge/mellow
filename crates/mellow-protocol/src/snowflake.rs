use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::canonical_decimal::parse_canonical_decimal_u64;

/// Unix time in milliseconds of 2026-01-01T00:00:00Z, the origin of every Mellow id timestamp.
pub const MELLOW_EPOCH_UNIX_MILLISECONDS: u64 = 1_767_225_600_000;

const TIMESTAMP_BITS: u32 = 42;
const SEQUENCE_BITS: u32 = 10;
const RESERVED_BITS: u32 = 12;

/// Largest millisecond offset from [`MELLOW_EPOCH_UNIX_MILLISECONDS`] an id can hold (about 139 years).
pub const MAXIMUM_TIMESTAMP_MILLISECONDS_SINCE_EPOCH: u64 = (1 << TIMESTAMP_BITS) - 1;
/// Largest per-millisecond sequence number an id can hold.
pub const MAXIMUM_SEQUENCE: u16 = (1 << SEQUENCE_BITS) - 1;

/// A 64-bit, time-ordered identifier (ADR-0004).
///
/// Layout from the most significant bit: 42 bits of milliseconds since the Mellow epoch,
/// 10 bits of sequence, 12 reserved bits (always zero for now).
/// Serialized as a decimal string in JSON so clients never lose precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Snowflake(u64);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SnowflakeError {
    #[error("timestamp {0} ms since the Mellow epoch exceeds the 42-bit limit")]
    TimestampOutOfRange(u64),
    #[error("sequence {0} exceeds the 10-bit limit")]
    SequenceOutOfRange(u16),
    #[error("id is not a canonical decimal unsigned 64-bit integer")]
    NotADecimalInteger,
}

impl Snowflake {
    /// Builds an id from its parts.
    ///
    /// # Errors
    /// Returns an error if the timestamp exceeds 42 bits or the sequence exceeds 10 bits.
    pub fn from_parts(
        milliseconds_since_epoch: u64,
        sequence: u16,
    ) -> Result<Self, SnowflakeError> {
        if milliseconds_since_epoch > MAXIMUM_TIMESTAMP_MILLISECONDS_SINCE_EPOCH {
            return Err(SnowflakeError::TimestampOutOfRange(
                milliseconds_since_epoch,
            ));
        }
        if sequence > MAXIMUM_SEQUENCE {
            return Err(SnowflakeError::SequenceOutOfRange(sequence));
        }
        Ok(Self(
            (milliseconds_since_epoch << (SEQUENCE_BITS + RESERVED_BITS))
                | (u64::from(sequence) << RESERVED_BITS),
        ))
    }

    /// Wraps a raw value read from storage.
    #[must_use]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn milliseconds_since_epoch(self) -> u64 {
        self.0 >> (SEQUENCE_BITS + RESERVED_BITS)
    }

    #[must_use]
    pub const fn unix_milliseconds(self) -> u64 {
        self.milliseconds_since_epoch() + MELLOW_EPOCH_UNIX_MILLISECONDS
    }

    #[must_use]
    pub const fn sequence(self) -> u16 {
        // Masked to 10 bits, so the cast cannot truncate.
        #[allow(clippy::cast_possible_truncation)]
        let sequence = ((self.0 >> RESERVED_BITS) & MAXIMUM_SEQUENCE as u64) as u16;
        sequence
    }
}

impl fmt::Display for Snowflake {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for Snowflake {
    type Err = SnowflakeError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        parse_canonical_decimal_u64(text)
            .map(Self)
            .ok_or(SnowflakeError::NotADecimalInteger)
    }
}

impl Serialize for Snowflake {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Snowflake {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_round_trip() {
        let id = Snowflake::from_parts(123_456_789, 42).unwrap();
        assert_eq!(id.milliseconds_since_epoch(), 123_456_789);
        assert_eq!(id.sequence(), 42);
        assert_eq!(
            id.unix_milliseconds(),
            MELLOW_EPOCH_UNIX_MILLISECONDS + 123_456_789
        );
        assert_eq!(id.raw() & ((1 << RESERVED_BITS) - 1), 0);
    }

    #[test]
    fn maximum_parts_are_accepted() {
        let id =
            Snowflake::from_parts(MAXIMUM_TIMESTAMP_MILLISECONDS_SINCE_EPOCH, MAXIMUM_SEQUENCE)
                .unwrap();
        assert_eq!(
            id.milliseconds_since_epoch(),
            MAXIMUM_TIMESTAMP_MILLISECONDS_SINCE_EPOCH
        );
        assert_eq!(id.sequence(), MAXIMUM_SEQUENCE);
    }

    #[test]
    fn out_of_range_parts_are_rejected() {
        assert_eq!(
            Snowflake::from_parts(MAXIMUM_TIMESTAMP_MILLISECONDS_SINCE_EPOCH + 1, 0),
            Err(SnowflakeError::TimestampOutOfRange(
                MAXIMUM_TIMESTAMP_MILLISECONDS_SINCE_EPOCH + 1
            ))
        );
        assert_eq!(
            Snowflake::from_parts(0, MAXIMUM_SEQUENCE + 1),
            Err(SnowflakeError::SequenceOutOfRange(MAXIMUM_SEQUENCE + 1))
        );
    }

    #[test]
    fn ids_sort_by_time_then_sequence() {
        let earlier = Snowflake::from_parts(1_000, MAXIMUM_SEQUENCE).unwrap();
        let later = Snowflake::from_parts(1_001, 0).unwrap();
        let later_next = Snowflake::from_parts(1_001, 1).unwrap();
        assert!(earlier < later && later < later_next);
    }

    #[test]
    fn json_uses_decimal_string() {
        let id = Snowflake::from_raw(u64::MAX);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"18446744073709551615\"");
        assert_eq!(serde_json::from_str::<Snowflake>(&json).unwrap(), id);
    }

    #[test]
    fn non_canonical_strings_are_rejected() {
        for text in [
            "",
            "+1",
            "-1",
            " 1",
            "1.0",
            "0x10",
            "01",
            "18446744073709551616",
        ] {
            assert_eq!(
                text.parse::<Snowflake>(),
                Err(SnowflakeError::NotADecimalInteger),
                "{text:?}"
            );
        }
        assert!(
            serde_json::from_str::<Snowflake>("42").is_err(),
            "numbers must be strings"
        );
    }
}
