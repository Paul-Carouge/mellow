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

/// Current-password checks (password change, username change) per session: a stolen session must
/// not be able to guess the password quickly. Per session, not per user, so a thief cannot use up
/// the budget of the owner's own devices and keep them from changing the password.
pub(crate) const PASSWORD_CHECKS_PER_SESSION: RateLimit = RateLimit {
    maximum_events: 10,
    window: Duration::from_mins(15),
};
/// Current-password checks per user, across all sessions. Only checks a session's own budget
/// allowed count here, so one session cannot fill it alone.
pub(crate) const PASSWORD_CHECKS_PER_USER: RateLimit = RateLimit {
    maximum_events: 50,
    window: Duration::from_mins(15),
};
/// Username and display name changes per session: each one is fanned out to every shared guild.
/// Per session, so a stolen session cannot use up the owner's budget.
pub(crate) const PROFILE_WRITES_PER_SESSION: RateLimit = RateLimit {
    maximum_events: 10,
    window: Duration::from_secs(60),
};
/// Username and display name changes per user, across all sessions, so an account cannot multiply
/// the fan-out by opening sessions. Only changes a session's own budget allowed count here, so one
/// session cannot fill it alone.
pub(crate) const PROFILE_WRITES_PER_USER: RateLimit = RateLimit {
    maximum_events: 30,
    window: Duration::from_secs(60),
};

/// Invite redemptions per client address: invite codes must not be guessable by brute force.
pub(crate) const INVITE_ATTEMPTS_PER_ADDRESS: RateLimit = RateLimit {
    maximum_events: 30,
    window: Duration::from_secs(60),
};

/// Guild, channel and invite writes per user, so quotas fill slowly and cheaply.
pub(crate) const GUILD_WRITES_PER_USER: RateLimit = RateLimit {
    maximum_events: 30,
    window: Duration::from_secs(60),
};

/// Message writes (send, edit, delete) per user: a burst is fine, a flood is not.
pub(crate) const MESSAGE_WRITES_PER_USER: RateLimit = RateLimit {
    maximum_events: 10,
    window: Duration::from_secs(10),
};
/// Sustained message writes per user, so one account cannot fill the disk over a day.
pub(crate) const MESSAGE_WRITES_PER_USER_PER_HOUR: RateLimit = RateLimit {
    maximum_events: 600,
    window: Duration::from_hours(1),
};
/// Messages per channel, whoever sends them (slows coordinated floods).
pub(crate) const MESSAGES_PER_CHANNEL: RateLimit = RateLimit {
    maximum_events: 30,
    window: Duration::from_secs(10),
};
/// Read-state acknowledgements per user; clients send one per channel switch or scroll.
pub(crate) const READ_STATE_WRITES_PER_USER: RateLimit = RateLimit {
    maximum_events: 30,
    window: Duration::from_secs(10),
};

/// Conversations opened per user: each one checks shared guilds and may create a channel.
pub(crate) const DIRECT_MESSAGE_OPENS_PER_USER: RateLimit = RateLimit {
    maximum_events: 10,
    window: Duration::from_secs(60),
};
/// The daily ceiling behind it: every new conversation is a stored channel, kept for good.
pub(crate) const DIRECT_MESSAGE_OPENS_PER_USER_PER_DAY: RateLimit = RateLimit {
    maximum_events: 200,
    window: Duration::from_hours(24),
};

/// Attachment uploads per user (ADR-0020): each one is up to the instance's file limit on disk.
pub(crate) const ATTACHMENT_UPLOADS_PER_USER: RateLimit = RateLimit {
    maximum_events: 10,
    window: Duration::from_mins(1),
};
/// Sustained uploads per user, so one account cannot fill the disk over a day.
pub(crate) const ATTACHMENT_UPLOADS_PER_USER_PER_HOUR: RateLimit = RateLimit {
    maximum_events: 100,
    window: Duration::from_hours(1),
};

/// Typing indicators per user; clients send one every few seconds while typing.
pub(crate) const TYPING_INDICATORS_PER_USER: RateLimit = RateLimit {
    maximum_events: 10,
    window: Duration::from_secs(10),
};

/// Gateway WebSocket openings per client address (each holds a socket for up to 10 s unidentified).
pub(crate) const GATEWAY_CONNECTIONS_PER_ADDRESS: RateLimit = RateLimit {
    maximum_events: 10,
    window: Duration::from_secs(10),
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
