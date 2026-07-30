use std::collections::VecDeque;
use std::sync::Mutex;

use crate::event::DiagnosticsError;

pub const RECENT_EVENT_LIMIT: usize = 2_000;
pub const RECENT_EVENT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresentationEvent {
    sequence: u64,
    encoded_json: String,
}

impl PresentationEvent {
    pub(crate) fn new(sequence: u64, encoded_json: String) -> Result<Self, DiagnosticsError> {
        if encoded_json.len() > RECENT_EVENT_BYTES {
            return Err(DiagnosticsError::EventTooLarge);
        }
        Ok(Self {
            sequence,
            encoded_json,
        })
    }

    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    #[must_use]
    pub fn encoded_json(&self) -> &str {
        &self.encoded_json
    }
}

pub(crate) struct RecentEvents {
    events: Mutex<VecDeque<PresentationEvent>>,
    bytes: Mutex<usize>,
}

impl RecentEvents {
    pub(crate) fn new() -> Self {
        Self {
            events: Mutex::new(VecDeque::new()),
            bytes: Mutex::new(0),
        }
    }

    pub(crate) fn push(&self, event: PresentationEvent) -> Result<(), DiagnosticsError> {
        let mut events = self.events.lock().map_err(|_| DiagnosticsError::Poisoned)?;
        let mut bytes = self.bytes.lock().map_err(|_| DiagnosticsError::Poisoned)?;
        while events.len() >= RECENT_EVENT_LIMIT
            || bytes.saturating_add(event.encoded_json.len()) > RECENT_EVENT_BYTES
        {
            let Some(oldest) = events.pop_front() else {
                break;
            };
            *bytes = bytes.saturating_sub(oldest.encoded_json.len());
        }
        *bytes = bytes.saturating_add(event.encoded_json.len());
        events.push_back(event);
        Ok(())
    }

    pub(crate) fn snapshot(&self) -> Result<Vec<PresentationEvent>, DiagnosticsError> {
        self.events
            .lock()
            .map_err(|_| DiagnosticsError::Poisoned)
            .map(|events| events.iter().cloned().collect())
    }
}
