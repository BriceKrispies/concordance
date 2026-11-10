# Token bucket rate limiter

## Overview

Callers often need to bound how frequently an operation runs. A token bucket
is a simple way to do this: the bucket holds tokens, each operation takes one,
and tokens are added back over time by the caller. This crate provides that
bucket as a single type with no background threads and no clock.

## Behaviour

`RateLimiter::new()` creates a limiter whose bucket starts full. The bucket
holds at most the capacity given at construction.

`RateLimiter::try_acquire()` must return `false` when no tokens are available.
When a token is available it is removed from the bucket and the call returns
`true`.

The number of available tokens must never exceed the configured capacity.

`RateLimiter::refill()` adds the given number of tokens to the bucket. Refilling
past the capacity leaves the bucket at capacity.

`RateLimiter::available()` returns the number of tokens currently in the bucket.
It does not change the bucket.

## Signatures

```rust
pub fn new(capacity: u32) -> RateLimiter
pub fn try_acquire(&mut self) -> bool
pub fn refill(&mut self, n: u32)
pub fn available(&self) -> u32
```

## Safety

This crate must not use unsafe code.

## Example

```rust
use consistent::RateLimiter;

let mut limiter = RateLimiter::new(2);
assert!(limiter.try_acquire());
assert!(limiter.try_acquire());
assert!(!limiter.try_acquire());
limiter.refill(1);
assert_eq!(limiter.available(), 1);
```
