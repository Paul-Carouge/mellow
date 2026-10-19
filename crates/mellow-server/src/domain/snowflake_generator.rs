use std::sync::Mutex;

use mellow_protocol::{
    MAXIMUM_SEQUENCE, MELLOW_EPOCH_UNIX_MILLISECONDS, Snowflake, SnowflakeError,
};

/// Issues strictly increasing ids within this process (ADR-0004). The caller passes the current
/// time, so tests are deterministic. A separate process (e.g. `admin create-user` while `serve`
/// runs) has its own generator; a same-millisecond collision is then rejected by the primary key.
#[derive(Debug, Default)]
pub struct SnowflakeGenerator {
    last_issued: Mutex<LastIssued>,
}

#[derive(Debug, Default, Clone, Copy)]
struct LastIssued {
    milliseconds_since_epoch: u64,
    sequence: u16,
}

impl SnowflakeGenerator {
    /// Returns an id greater than every id issued before. If the clock goes backwards or more
    /// than 1 024 ids are requested within one millisecond, the id borrows from the next
    /// millisecond instead of repeating or going backwards.
    ///
    /// # Errors
    /// Fails for clocks before the Mellow epoch (2026) or beyond the year ~2165.
    pub fn next_id(&self, now_unix_milliseconds: u64) -> Result<Snowflake, SnowflakeError> {
        let now_since_epoch = now_unix_milliseconds
            .checked_sub(MELLOW_EPOCH_UNIX_MILLISECONDS)
            .ok_or(SnowflakeError::TimestampOutOfRange(0))?;
        // A poisoned lock only means another thread panicked mid-update; the counter is still valid.
        let mut last = self
            .last_issued
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (milliseconds_since_epoch, sequence) =
            if now_since_epoch > last.milliseconds_since_epoch {
                (now_since_epoch, 0)
            } else if last.sequence < MAXIMUM_SEQUENCE {
                (last.milliseconds_since_epoch, last.sequence + 1)
            } else {
                (last.milliseconds_since_epoch + 1, 0)
            };
        let id = Snowflake::from_parts(milliseconds_since_epoch, sequence)?;
        *last = LastIssued {
            milliseconds_since_epoch,
            sequence,
        };
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOME_TIME: u64 = MELLOW_EPOCH_UNIX_MILLISECONDS + 1_000;

    #[test]
    fn ids_increase_within_a_millisecond_across_milliseconds_and_when_the_clock_goes_back() {
        let generator = SnowflakeGenerator::default();
        let first = generator.next_id(SOME_TIME).unwrap();
        let same_millisecond = generator.next_id(SOME_TIME).unwrap();
        let clock_went_back = generator.next_id(SOME_TIME - 500).unwrap();
        let later = generator.next_id(SOME_TIME + 10).unwrap();
        assert!(first < same_millisecond);
        assert!(same_millisecond < clock_went_back);
        assert!(clock_went_back < later);
        assert_eq!(later.unix_milliseconds(), SOME_TIME + 10);
    }

    #[test]
    fn a_clock_before_the_mellow_epoch_is_an_error() {
        let generator = SnowflakeGenerator::default();
        assert!(
            generator
                .next_id(MELLOW_EPOCH_UNIX_MILLISECONDS - 1)
                .is_err()
        );
        assert!(generator.next_id(0).is_err());
    }

    #[test]
    fn sequence_overflow_borrows_the_next_millisecond() {
        let generator = SnowflakeGenerator::default();
        let ids: Vec<_> = (0..=u32::from(MAXIMUM_SEQUENCE) + 1)
            .map(|_| generator.next_id(SOME_TIME).unwrap())
            .collect();
        assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
        let last = ids.last().unwrap();
        assert_eq!(last.unix_milliseconds(), SOME_TIME + 1);
        assert_eq!(last.sequence(), 0);
    }
}
