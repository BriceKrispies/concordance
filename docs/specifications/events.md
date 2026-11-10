# Event streaming

Concordance is streaming-first: every long-running operation and every committed
state change is published as a typed event, and every consumer (browser, `watch
--jsonl`, the scan progress printer) reads the same stream.

## Event bus

The bus is `concordance_events::EventBus`. Every published event is wrapped in an
`Envelope` carrying a sequence number, the model revision and a timestamp.

- Sequence numbers MUST be strictly increasing and assigned by `EventBus::publish()` only.
- The bus MUST keep at most its configured capacity of recent envelopes; memory use must not grow with the number or speed of subscribers.
- A subscriber that falls behind the replay window MUST receive `Next::Lagged` instead of a gap.
- `EventBus::can_replay_from()` must return `false` for a cursor that is newer than the last published event.
- The bus must not block publishers on slow subscribers.

```rust
pub fn can_replay_from(&self, cursor: u64) -> bool
pub fn subscribe(&self, cursor: Option<u64>) -> Subscription
```

## Server-Sent Events

The browser transport is SSE at `/api/v1/events`.

- The first message of every stream MUST be a `hello` event.
- A client that reconnects with a replayable cursor must receive every event after that cursor exactly once and in order.
- A client whose cursor cannot be replayed MUST receive a `resync` event and reload its snapshots.
- The server should send a keep-alive comment at least every 15 seconds.

## Snapshot consistency

Snapshot responses report the sequence number they are consistent with.
`Shared::read()` takes the model read lock and reads `EventBus::last_seq()` while
holding it; state-change events are only published while the engine holds the
write lock, so a snapshot and its `seq` always agree.

## Background

Server-Sent Events were chosen over WebSockets because the traffic is one-way and
browsers reconnect automatically; the cursor makes reconnection lossless.
