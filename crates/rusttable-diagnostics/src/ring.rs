use std::collections::VecDeque;
use std::sync::{Mutex, mpsc};

use crate::event::DiagnosticRecord;

pub const MAX_RECENT_EVENTS: usize = 2_000;
pub const MAX_RECENT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct RecentEvent {
    pub sequence: u64,
    pub timestamp_unix_ms: u128,
    pub record: DiagnosticRecord,
}

struct Inner {
    entries: VecDeque<(usize, RecentEvent)>,
    bytes: usize,
    subscribers: Vec<mpsc::SyncSender<RecentEvent>>,
}

pub(crate) struct RecentRing {
    inner: Mutex<Inner>,
}

impl RecentRing {
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                entries: VecDeque::new(),
                bytes: 0,
                subscribers: Vec::new(),
            }),
        }
    }

    pub(crate) fn push(&self, event: &RecentEvent) {
        let size = encoded_size(event);
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        inner.bytes = inner.bytes.saturating_add(size);
        inner.entries.push_back((size, event.clone()));
        while inner.entries.len() > MAX_RECENT_EVENTS || inner.bytes > MAX_RECENT_BYTES {
            if let Some((size, _)) = inner.entries.pop_front() {
                inner.bytes = inner.bytes.saturating_sub(size);
            } else {
                break;
            }
        }
        inner
            .subscribers
            .retain(|subscriber| subscriber.try_send(event.clone()).is_ok());
    }

    pub(crate) fn snapshot(&self) -> Vec<RecentEvent> {
        self.inner.lock().map_or_else(
            |_| Vec::new(),
            |inner| {
                inner
                    .entries
                    .iter()
                    .map(|(_, event)| event.clone())
                    .collect()
            },
        )
    }

    pub(crate) fn subscribe(&self) -> RecentSubscription {
        let (sender, receiver) = mpsc::sync_channel(64);
        if let Ok(mut inner) = self.inner.lock() {
            inner.subscribers.push(sender);
        }
        RecentSubscription { receiver }
    }
}

fn encoded_size(event: &RecentEvent) -> usize {
    64 + event.record.operation.len()
        + event.record.code.as_str().len()
        + event
            .record
            .fields
            .iter()
            .map(|field| field.name().len() + field.value().encoded_len())
            .sum::<usize>()
}

pub struct RecentSubscription {
    receiver: mpsc::Receiver<RecentEvent>,
}

impl RecentSubscription {
    /// Receives the next queued event without blocking.
    ///
    /// # Errors
    ///
    /// Returns the standard channel error when no event is ready or the ring is closed.
    pub fn try_recv(&self) -> Result<RecentEvent, mpsc::TryRecvError> {
        self.receiver.try_recv()
    }
}
