//! A small in-memory rate limiter (fixed windows), for login and password-reset attempts.
//!
//! In-memory is enough: the API runs as a single instance, and losing the counters on a
//! restart only gives an attacker a handful of extra attempts.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Above this many tracked keys, expired windows are purged.
const PRUNE_THRESHOLD: usize = 10_000;

struct Window {
    started: Instant,
    count: u32,
}

#[derive(Default)]
pub struct RateLimiter {
    windows: Mutex<HashMap<String, Window>>,
}

impl RateLimiter {
    /// Counts one attempt for `key`. Returns `false` when more than `max` attempts were made
    /// within `window`.
    pub fn hit(&self, key: &str, max: u32, window: Duration) -> bool {
        self.hit_at(key, max, window, Instant::now())
    }

    fn hit_at(&self, key: &str, max: u32, window: Duration, now: Instant) -> bool {
        let mut windows = self.windows.lock().expect("rate limiter lock poisoned");
        if windows.len() > PRUNE_THRESHOLD {
            windows.retain(|_, w| now.duration_since(w.started) < window);
        }
        let entry = windows.entry(key.to_owned()).or_insert(Window {
            started: now,
            count: 0,
        });
        if now.duration_since(entry.started) >= window {
            entry.started = now;
            entry.count = 0;
        }
        entry.count = entry.count.saturating_add(1);
        entry.count <= max
    }

    /// Forgets `key` (after a successful login).
    pub fn reset(&self, key: &str) {
        self.windows
            .lock()
            .expect("rate limiter lock poisoned")
            .remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: Duration = Duration::from_secs(60);

    #[test]
    fn allows_up_to_max_then_blocks() {
        let limiter = RateLimiter::default();
        let now = Instant::now();
        for _ in 0..3 {
            assert!(limiter.hit_at("k", 3, WINDOW, now));
        }
        assert!(!limiter.hit_at("k", 3, WINDOW, now));
        assert!(!limiter.hit_at("k", 3, WINDOW, now + Duration::from_secs(59)));
    }

    #[test]
    fn keys_are_independent() {
        let limiter = RateLimiter::default();
        let now = Instant::now();
        assert!(limiter.hit_at("a", 1, WINDOW, now));
        assert!(!limiter.hit_at("a", 1, WINDOW, now));
        assert!(limiter.hit_at("b", 1, WINDOW, now));
    }

    #[test]
    fn window_expires() {
        let limiter = RateLimiter::default();
        let now = Instant::now();
        assert!(limiter.hit_at("k", 1, WINDOW, now));
        assert!(!limiter.hit_at("k", 1, WINDOW, now));
        assert!(limiter.hit_at("k", 1, WINDOW, now + WINDOW));
    }

    #[test]
    fn reset_clears_the_counter() {
        let limiter = RateLimiter::default();
        assert!(limiter.hit("k", 1, WINDOW));
        assert!(!limiter.hit("k", 1, WINDOW));
        limiter.reset("k");
        assert!(limiter.hit("k", 1, WINDOW));
    }
}
