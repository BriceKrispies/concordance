//! A token bucket rate limiter.

/// A token bucket that hands out one token per acquisition.
pub struct RateLimiter {
    capacity: u32,
    tokens: u32,
}

impl RateLimiter {
    /// Creates a limiter whose bucket starts full.
    pub fn new(capacity: u32) -> RateLimiter {
        RateLimiter {
            capacity,
            tokens: capacity,
        }
    }

    /// Takes one token if one is available.
    pub fn try_acquire(&mut self) -> bool {
        if self.tokens == 0 {
            return false;
        }
        self.tokens -= 1;
        true
    }

    /// Adds `n` tokens to the bucket, never exceeding the capacity.
    pub fn refill(&mut self, n: u32) {
        self.tokens = self.tokens.saturating_add(n).min(self.capacity);
    }

    /// Returns the number of tokens currently in the bucket.
    pub fn available(&self) -> u32 {
        self.tokens
    }
}
