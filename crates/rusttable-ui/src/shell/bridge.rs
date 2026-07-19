use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use super::model::UiMessage;
use super::subscriptions::SubscriptionIdentity;
use iced::Subscription;

const DEFAULT_CAPACITY: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub enum ServiceEvent {
    Ready,
    Degraded,
    Failed,
    Progress { job: u64, fraction: f32 },
    Finished { job: u64 },
}

impl ServiceEvent {
    #[must_use]
    pub const fn progress(job: u64, fraction: f32) -> Self {
        Self::Progress { job, fraction }
    }

    #[must_use]
    pub const fn finished(job: u64) -> Self {
        Self::Finished { job }
    }

    #[must_use]
    pub const fn kind(&self) -> ServiceEventKind {
        match self {
            Self::Ready => ServiceEventKind::Ready,
            Self::Degraded => ServiceEventKind::Degraded,
            Self::Failed => ServiceEventKind::Failed,
            Self::Progress { job, fraction } => ServiceEventKind::Progress {
                job: *job,
                fraction: *fraction,
            },
            Self::Finished { job } => ServiceEventKind::Finished { job: *job },
        }
    }

    const fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Ready | Self::Degraded | Self::Failed | Self::Finished { .. }
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ServiceEventKind {
    Ready,
    Degraded,
    Failed,
    Progress { job: u64, fraction: f32 },
    Finished { job: u64 },
}

#[derive(Debug, Clone)]
pub struct BoundedServiceBridge {
    queue: Arc<Mutex<VecDeque<ServiceEvent>>>,
    capacity: usize,
}

impl BoundedServiceBridge {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            queue: Arc::new(Mutex::new(VecDeque::with_capacity(capacity.max(1)))),
            capacity: capacity.max(1),
        }
    }

    #[must_use]
    pub fn with_default_capacity() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }

    pub fn push(&self, event: ServiceEvent) {
        let Ok(mut queue) = self.queue.lock() else {
            return;
        };
        if let ServiceEvent::Progress { job, fraction } = event {
            if let Some(existing) = queue.iter_mut().find(|existing| {
                matches!(existing, ServiceEvent::Progress { job: existing_job, .. } if *existing_job == job)
            }) {
                *existing = ServiceEvent::Progress { job, fraction };
                return;
            }
            if queue.len() >= self.capacity
                && let Some(index) = queue.iter().position(|event| !event.is_terminal())
            {
                let _ = queue.remove(index);
            }
            if queue.len() < self.capacity {
                queue.push_back(ServiceEvent::Progress { job, fraction });
            }
            return;
        }
        if queue.len() >= self.capacity
            && let Some(index) = queue.iter().position(|queued| !queued.is_terminal())
        {
            let _ = queue.remove(index);
        }
        if queue.len() >= self.capacity {
            let _ = queue.pop_front();
        }
        queue.push_back(event);
    }

    #[must_use]
    pub fn drain(&self) -> Vec<ServiceEvent> {
        self.queue
            .lock()
            .map(|mut queue| queue.drain(..).collect())
            .unwrap_or_default()
    }
}

/// Adapts a typed service stream into keyed Iced subscription messages.
pub fn service_subscription(
    identity: SubscriptionIdentity,
    build: fn() -> iced::futures::stream::BoxStream<'static, ServiceEvent>,
) -> Subscription<UiMessage> {
    Subscription::run(build)
        .map(UiMessage::Service)
        .with(identity)
        .map(|(_, message)| message)
}

#[cfg(test)]
mod tests {
    use super::{BoundedServiceBridge, ServiceEvent};

    #[test]
    fn terminal_events_survive_full_queue() {
        let bridge = BoundedServiceBridge::new(1);
        bridge.push(ServiceEvent::progress(1, 0.1));
        bridge.push(ServiceEvent::finished(1));
        assert_eq!(bridge.drain(), [ServiceEvent::finished(1)]);
    }
}
