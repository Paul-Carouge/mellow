//! In-memory fixed-window rate limiting for authentication (ARCHITECTURE.md §4.7).
//! shortcut: per process and lost on restart; fine for a single-node instance (ADR-0002).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Above this many tracked keys, expired windows are pruned.
const PRUNE_THRESHOLD_KEYS: usize = 10_000;
/// Hard ceiling: past it (even after pruning) new keys are refused, so memory stays bounded
/// whatever an attacker sends. Keys are short and fixed-size (see `account_routes`), so this is a
/// few megabytes at most.
const MAXIMUM_TRACKED_KEYS: usize = 100_000;
/// Pruning is O(n) under the lock; doing it at most this often keeps a full table cheap.
const MINIMUM_INTERVAL_BETWEEN_PRUNES: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RateLimit {
    pub(crate) maximum_events: u32,
    pub(crate) window: Duration,
}

/// Registration and login attempts per client address.
pub(crate) const AUTHENTICATION_ATTEMPTS_PER_ADDRESS: RateLimit = RateLimit {
    maximum_events: 20,
    window: Duration::from_secs(60),
};
/// Login attempts on one account from one address: the brute-force limit.
pub(crate) const LOGIN_ATTEMPTS_PER_ACCOUNT_AND_ADDRESS: RateLimit = RateLimit {
    maximum_events: 10,
    window: Duration::from_mins(15),
};
/// Login attempts on one account from all addresses: slows distributed credential stuffing.
/// Deliberately much higher than the per-address limit, so locking a victim out takes many
/// requests (OWASP: prefer throttling to hard lockout).
pub(crate) const LOGIN_ATTEMPTS_PER_ACCOUNT: RateLimit = RateLimit {
    maximum_events: 100,
    window: Duration::from_mins(15),
};

#[derive(Debug, Default)]
pub(crate) struct RateLimiter {
    state: Mutex<LimiterState>,
}

#[derive(Debug, Default)]
struct LimiterState {
    windows: HashMap<String, Window>,
    last_pruned_at: Option<Instant>,
}

#[derive(Debug, Clone, Copy)]
struct Window {
    started_at: Instant,
    length: Duration,
    events: u32,
}

impl Window {
    fn has_ended(&self, now: Instant) -> bool {
        now.duration_since(self.started_at) >= self.length
    }
}

impl RateLimiter {
    /// Records one event for `key` and returns whether it is still within `limit`. Counting
    /// happens before the guarded work, so concurrent requests cannot all slip under the limit.
    pub(crate) fn record_event(&self, key: &str, limit: RateLimit, now: Instant) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !state.windows.contains_key(key) && !state.make_room(now) {
            return false;
        }
        let window = state.windows.entry(key.to_owned()).or_insert(Window {
            started_at: now,
            length: limit.window,
            events: 0,
        });
        if window.has_ended(now) {
            *window = Window {
                started_at: now,
                length: limit.window,
                events: 0,
            };
        }
        window.events = window.events.saturating_add(1);
        window.events <= limit.maximum_events
    }
}

impl LimiterState {
    /// Returns whether a new key may be tracked.
    fn make_room(&mut self, now: Instant) -> bool {
        let may_prune = self
            .last_pruned_at
            .is_none_or(|last| now.duration_since(last) >= MINIMUM_INTERVAL_BETWEEN_PRUNES);
        if self.windows.len() >= PRUNE_THRESHOLD_KEYS && may_prune {
            // Each window expires on its own length, so short address windows never evict
            // longer per-account ones.
            self.windows.retain(|_, window| !window.has_ended(now));
            self.last_pruned_at = Some(now);
        }
        self.windows.len() < MAXIMUM_TRACKED_KEYS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHORT: RateLimit = RateLimit {
        maximum_events: 2,
        window: Duration::from_secs(60),
    };
    const LONG: RateLimit = RateLimit {
        maximum_events: 1,
        window: Duration::from_mins(15),
    };

    #[test]
    fn events_beyond_the_limit_are_refused_until_the_window_ends() {
        let limiter = RateLimiter::default();
        let start = Instant::now();
        assert!(limiter.record_event("ip:1", SHORT, start));
        assert!(limiter.record_event("ip:1", SHORT, start));
        assert!(!limiter.record_event("ip:1", SHORT, start + Duration::from_secs(59)));
        assert!(
            limiter.record_event("ip:2", SHORT, start),
            "keys are independent"
        );
        assert!(limiter.record_event("ip:1", SHORT, start + SHORT.window));
    }

    #[test]
    fn pruning_keeps_each_window_for_its_own_length() {
        let limiter = RateLimiter::default();
        let start = Instant::now();
        assert!(limiter.record_event("account", LONG, start));
        for index in 0..PRUNE_THRESHOLD_KEYS {
            limiter.record_event(&format!("ip:{index}"), SHORT, start);
        }
        // Two minutes later the short windows have ended and a prune runs; the long one must survive.
        let later = start + Duration::from_secs(120);
        assert!(limiter.record_event("ip:new", SHORT, later));
        assert!(
            !limiter.record_event("account", LONG, later),
            "account window was pruned too early"
        );
    }

    #[test]
    fn new_keys_are_refused_once_the_table_is_full_of_live_windows() {
        let limiter = RateLimiter::default();
        let start = Instant::now();
        for index in 0..MAXIMUM_TRACKED_KEYS {
            assert!(limiter.record_event(&format!("ip:{index}"), LONG, start));
        }
        assert!(!limiter.record_event("ip:one-too-many", LONG, start));
        assert!(
            !limiter.record_event("ip:0", LONG, start),
            "existing keys still count"
        );
    }
}
