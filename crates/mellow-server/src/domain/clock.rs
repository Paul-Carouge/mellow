use std::time::{SystemTime, UNIX_EPOCH};

/// Current Unix time in milliseconds; 0 if the system clock is before 1970.
#[must_use]
pub fn now_unix_milliseconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}
