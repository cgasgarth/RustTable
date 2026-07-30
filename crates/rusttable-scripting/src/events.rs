use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::dto::EventDto;
use crate::errors::{ErrorCode, ScriptError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EventKind {
    CatalogChanged,
    JobProgress,
    PreviewProgress,
    Notification,
}

impl EventKind {
    #[must_use]
    pub const fn coalescible(self) -> bool {
        matches!(self, Self::PreviewProgress | Self::JobProgress)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventRecord {
    pub sequence: u64,
    pub kind: EventKind,
    pub payload: EventDto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EventFilter {
    pub kind: Option<EventKind>,
}

impl EventFilter {
    #[must_use]
    pub fn matches(self, event: &EventRecord) -> bool {
        self.kind.is_none_or(|kind| kind == event.kind)
    }
}

#[derive(Debug, Clone)]
pub struct EventSubscription {
    cursor: u64,
    filter: EventFilter,
}

impl EventSubscription {
    #[must_use]
    pub const fn cursor(&self) -> u64 {
        self.cursor
    }
}

#[derive(Debug, Clone)]
pub struct EventBus {
    queue: VecDeque<EventRecord>,
    next_sequence: u64,
    missed: u64,
    max_queue: usize,
    delivering: bool,
}

impl EventBus {
    #[must_use]
    pub fn new(max_queue: usize) -> Self {
        Self {
            queue: VecDeque::new(),
            next_sequence: 0,
            missed: 0,
            max_queue,
            delivering: false,
        }
    }

    #[must_use]
    pub fn subscribe(&self, filter: EventFilter) -> EventSubscription {
        EventSubscription {
            cursor: self.next_sequence,
            filter,
        }
    }

    /// # Errors
    ///
    /// Returns `EventBackpressure` when the queue is full or a coalescing entry is unavailable.
    pub fn publish(&mut self, kind: EventKind, payload: String) -> Result<u64, ScriptError> {
        self.next_sequence = self.next_sequence.saturating_add(1);
        if kind.coalescible() && self.queue.back().is_some_and(|event| event.kind == kind) {
            let Some(event) = self.queue.back_mut() else {
                return Err(ScriptError::new(
                    ErrorCode::EventBackpressure,
                    "coalescing queue entry disappeared",
                ));
            };
            event.sequence = self.next_sequence;
            event.payload.payload = payload;
            return Ok(event.sequence);
        }
        if self.queue.len() >= self.max_queue {
            self.missed = self.missed.saturating_add(1);
            return Err(ScriptError::new(
                ErrorCode::EventBackpressure,
                "event queue quota exceeded",
            ));
        }
        self.queue.push_back(EventRecord {
            sequence: self.next_sequence,
            kind,
            payload: EventDto {
                kind: format!("{kind:?}"),
                sequence: self.next_sequence,
                payload,
            },
        });
        Ok(self.next_sequence)
    }

    /// # Errors
    ///
    /// Returns `EventBackpressure` when delivery is already active.
    pub fn drain(
        &mut self,
        subscription: &mut EventSubscription,
    ) -> Result<Vec<EventRecord>, ScriptError> {
        if self.delivering {
            return Err(ScriptError::new(
                ErrorCode::EventBackpressure,
                "reentrant event delivery is prohibited",
            ));
        }
        self.delivering = true;
        let events = self
            .queue
            .iter()
            .filter(|event| {
                event.sequence > subscription.cursor && subscription.filter.matches(event)
            })
            .cloned()
            .collect::<Vec<_>>();
        if let Some(last) = self.queue.back() {
            subscription.cursor = last.sequence;
        }
        self.queue
            .retain(|event| event.sequence > subscription.cursor);
        self.delivering = false;
        Ok(events)
    }

    #[must_use]
    pub const fn missed(&self) -> u64 {
        self.missed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_are_ordered_and_progress_is_coalesced() {
        let mut bus = EventBus::new(4);
        let mut subscription = bus.subscribe(EventFilter::default());
        bus.publish(EventKind::JobProgress, "1".to_owned())
            .expect("event");
        bus.publish(EventKind::JobProgress, "2".to_owned())
            .expect("event");
        bus.publish(EventKind::CatalogChanged, "3".to_owned())
            .expect("event");
        let events = bus.drain(&mut subscription).expect("drain");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].payload.payload, "2");
        assert_eq!(events[1].sequence, 3);
    }

    #[test]
    fn queue_fails_closed() {
        let mut bus = EventBus::new(1);
        assert!(bus.publish(EventKind::Notification, "1".to_owned()).is_ok());
        assert_eq!(
            bus.publish(EventKind::Notification, "2".to_owned())
                .unwrap_err()
                .code,
            ErrorCode::EventBackpressure
        );
    }
}
