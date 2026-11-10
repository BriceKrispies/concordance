//! Central event distribution.
//!
//! One bounded ring of recent envelopes plus a `watch` channel carrying the latest
//! sequence number. Subscribers are just cursors: they read forward from their last
//! seen `seq`. This gives, with no per-subscriber buffers:
//! * ordered sequence numbers and replay from any cursor still in the ring,
//! * slow-client isolation: a slow reader never blocks publishers or other readers;
//!   if it falls behind the ring it gets [`Next::Lagged`] and must resynchronize
//!   from a snapshot,
//! * bounded memory regardless of subscriber count or speed.

use concordance_core::event::{Envelope, Event};
use concordance_core::{Revision, Timestamp};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

pub const DEFAULT_CAPACITY: usize = 8192;

struct Ring {
    next_seq: u64,
    revision: Revision,
    buf: VecDeque<Arc<Envelope>>,
    cap: usize,
}

/// Cloneable handle to the bus.
#[derive(Clone)]
pub struct EventBus {
    ring: Arc<Mutex<Ring>>,
    latest: watch::Sender<u64>,
    shutdown: watch::Sender<bool>,
}

/// Result of polling a subscription.
#[derive(Debug)]
pub enum Next {
    Event(Arc<Envelope>),
    /// The cursor fell out of the replay window. `latest` is the newest seq; the
    /// subscriber must reload a snapshot and continue from it.
    Lagged {
        latest: u64,
    },
    Closed,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (latest, _) = watch::channel(0);
        let (shutdown, _) = watch::channel(false);
        EventBus {
            ring: Arc::new(Mutex::new(Ring {
                next_seq: 1,
                revision: Revision(0),
                buf: VecDeque::with_capacity(capacity.min(1024)),
                cap: capacity.max(16),
            })),
            latest,
            shutdown,
        }
    }

    /// Start numbering after `seq` (used after restart so cursors stay monotonic).
    pub fn resume_after(&self, seq: u64, revision: Revision) {
        let mut r = self.ring.lock().unwrap();
        if r.next_seq <= seq {
            r.next_seq = seq + 1;
        }
        r.revision = revision;
        drop(r);
        self.latest.send_replace(seq);
    }

    /// Record the model revision stamped on subsequent events.
    pub fn set_revision(&self, revision: Revision) {
        self.ring.lock().unwrap().revision = revision;
    }

    pub fn publish(&self, event: Event) -> Arc<Envelope> {
        let env = {
            let mut r = self.ring.lock().unwrap();
            let env = Arc::new(Envelope {
                seq: r.next_seq,
                revision: r.revision,
                at: Timestamp::now(),
                event,
            });
            r.next_seq += 1;
            if r.buf.len() == r.cap {
                r.buf.pop_front();
            }
            r.buf.push_back(env.clone());
            env
        };
        self.latest.send_replace(env.seq);
        env
    }

    /// Sequence number of the most recently published event (0 if none).
    pub fn last_seq(&self) -> u64 {
        *self.latest.borrow()
    }

    /// Oldest seq still replayable, or `None` when the ring is empty.
    pub fn oldest_seq(&self) -> Option<u64> {
        self.ring.lock().unwrap().buf.front().map(|e| e.seq)
    }

    /// Subscribe starting after `cursor`. `None` = only events published from now on.
    pub fn subscribe(&self, cursor: Option<u64>) -> Subscription {
        Subscription {
            bus: self.clone(),
            cursor: cursor.unwrap_or_else(|| self.last_seq()),
            latest: self.latest.subscribe(),
            shutdown: self.shutdown.subscribe(),
        }
    }

    /// Can a subscriber at `cursor` replay without gaps? A cursor from the future
    /// (e.g. issued before a restart that lost unpersisted events) cannot.
    pub fn can_replay_from(&self, cursor: u64) -> bool {
        let r = self.ring.lock().unwrap();
        if cursor >= r.next_seq {
            return false;
        }
        match r.buf.front() {
            Some(front) => cursor + 1 >= front.seq,
            None => cursor + 1 == r.next_seq,
        }
    }

    /// Envelopes with `seq > after`, oldest first, at most `limit`. With `after = 0` the
    /// newest `limit` are returned (initial fill); otherwise the oldest `limit` after the
    /// cursor, so a client paging forward never skips events.
    pub fn recent(&self, after: u64, limit: usize) -> Vec<Arc<Envelope>> {
        let r = self.ring.lock().unwrap();
        if after == 0 {
            let mut v: Vec<Arc<Envelope>> = r.buf.iter().rev().take(limit).cloned().collect();
            v.reverse();
            return v;
        }
        r.buf
            .iter()
            .filter(|e| e.seq > after)
            .take(limit)
            .cloned()
            .collect()
    }

    /// Ends all subscriptions (they return [`Next::Closed`] once drained of nothing).
    pub fn close(&self) {
        self.shutdown.send_replace(true);
    }

    fn read_after(&self, cursor: u64) -> Result<Option<Arc<Envelope>>, u64> {
        let r = self.ring.lock().unwrap();
        let Some(front) = r.buf.front() else {
            return Ok(None);
        };
        let want = cursor + 1;
        if want < front.seq {
            return Err(r.next_seq - 1);
        }
        let idx = (want - front.seq) as usize;
        Ok(r.buf.get(idx).cloned())
    }
}

pub struct Subscription {
    bus: EventBus,
    cursor: u64,
    latest: watch::Receiver<u64>,
    shutdown: watch::Receiver<bool>,
}

impl Subscription {
    pub fn cursor(&self) -> u64 {
        self.cursor
    }

    /// Jump to the head (after a resync).
    pub fn reset_to(&mut self, seq: u64) {
        self.cursor = seq;
    }

    /// Non-blocking read of the next event, if already published.
    pub fn try_next(&mut self) -> Option<Next> {
        match self.bus.read_after(self.cursor) {
            Ok(Some(env)) => {
                self.cursor = env.seq;
                Some(Next::Event(env))
            }
            Ok(None) => None,
            Err(latest) => {
                self.cursor = latest;
                Some(Next::Lagged { latest })
            }
        }
    }

    /// Wait for the next event.
    pub async fn next(&mut self) -> Next {
        loop {
            if let Some(n) = self.try_next() {
                return n;
            }
            if *self.shutdown.borrow() {
                return Next::Closed;
            }
            tokio::select! {
                r = self.latest.changed() => if r.is_err() { return Next::Closed },
                r = self.shutdown.changed() => if r.is_err() || *self.shutdown.borrow() {
                    // Drain anything already published before reporting closure.
                    if let Some(n) = self.try_next() { return n; }
                    return Next::Closed;
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn warn(i: u32) -> Event {
        Event::SystemWarning {
            message: i.to_string(),
        }
    }

    #[tokio::test]
    async fn replay_from_cursor_is_gapless_and_lag_is_reported_not_buffered() {
        let bus = EventBus::new(16);
        for i in 0..10 {
            bus.publish(warn(i));
        }
        // A client that saw seq 4 reconnects: gets 5..=10 exactly once, in order.
        let mut sub = bus.subscribe(Some(4));
        let mut seen = vec![];
        while let Some(Next::Event(e)) = sub.try_next() {
            seen.push(e.seq);
        }
        assert_eq!(seen, (5..=10).collect::<Vec<_>>());

        // Overflow the ring past a slow subscriber's cursor.
        let mut slow = bus.subscribe(Some(10));
        for i in 0..40 {
            bus.publish(warn(i));
        }
        assert!(!bus.can_replay_from(10));
        match slow.next().await {
            Next::Lagged { latest } => assert_eq!(latest, 50),
            other => panic!("expected lag, got {other:?}"),
        }
        // After lag the subscriber continues from head with no duplicates.
        bus.publish(warn(99));
        match slow.next().await {
            Next::Event(e) => assert_eq!(e.seq, 51),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn waiting_subscriber_wakes_on_publish_and_on_close() {
        let bus = EventBus::new(16);
        let mut sub = bus.subscribe(None);
        let b2 = bus.clone();
        let h = tokio::spawn(async move {
            b2.publish(warn(1));
        });
        match sub.next().await {
            Next::Event(e) => assert_eq!(e.seq, 1),
            other => panic!("{other:?}"),
        }
        h.await.unwrap();
        bus.close();
        assert!(matches!(sub.next().await, Next::Closed));
    }
}
