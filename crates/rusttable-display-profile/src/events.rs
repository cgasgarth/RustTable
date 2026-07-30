use crate::{DisplayProfileId, DisplayProvider, MonitorId, ProfileSelection, SelectionStatus};
use std::collections::VecDeque;

pub const MAX_QUEUED_EVENTS: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayProfileEvent {
    MonitorAdded {
        monitor: MonitorId,
        generation: u64,
    },
    MonitorRemoved {
        monitor: MonitorId,
        generation: u64,
    },
    MonitorChanged {
        monitor: MonitorId,
        generation: u64,
    },
    ProfileHashChanged {
        monitor: MonitorId,
        previous: Option<DisplayProfileId>,
        current: Option<DisplayProfileId>,
        generation: u64,
    },
    ProviderAvailabilityChanged {
        monitor: MonitorId,
        provider: DisplayProvider,
        available: bool,
        generation: u64,
    },
    SelectionChanged {
        monitor: MonitorId,
        selection: ProfileSelection,
        status: SelectionStatus,
        generation: u64,
    },
    FallbackChanged {
        generation: u64,
    },
    OverrideChanged {
        monitor: MonitorId,
        generation: u64,
    },
}

#[derive(Debug, Clone)]
pub struct EventQueue {
    events: VecDeque<DisplayProfileEvent>,
}

impl Default for EventQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl EventQueue {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            events: VecDeque::new(),
        }
    }

    pub fn push(&mut self, event: DisplayProfileEvent) {
        if self.events.len() == MAX_QUEUED_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }

    #[must_use]
    pub fn drain(&mut self) -> Vec<DisplayProfileEvent> {
        self.events.drain(..).collect()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}
