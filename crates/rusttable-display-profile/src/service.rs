use crate::{
    DisplayProfileEvent, DisplayProfileId, DisplayProvider, EventQueue, HdrDescriptor,
    ManagedProfileStore, MonitorDescriptor, MonitorId, ProfileProbe, ProfileProbeFailure,
    ProviderAvailability, ProviderMonitor, StoredProfile,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileSelection {
    Override,
    OperatingSystem,
    UserFallback,
    Unprofiled,
}

pub type ProfileSource = ProfileSelection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaleReason {
    ProfileReadFailed,
    ProfileInvalid,
    ProfileUnsupported,
    ProfileOversized,
    ProfileChangedDuringRead,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DegradedReason {
    ProviderUnavailable,
    ProfileAbsent,
    PermissionDenied,
    ProfileUnreadable,
    MonitorUnresolved,
    MonitorRemoved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionStatus {
    Active,
    Stale(StaleReason),
    Degraded(DegradedReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HdrCapability {
    pub supported: bool,
    pub active: bool,
}

impl From<HdrDescriptor> for HdrCapability {
    fn from(value: HdrDescriptor) -> Self {
        Self {
            supported: value.supported,
            active: value.active,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DisplayProfileSnapshot {
    monitor: MonitorId,
    descriptor: MonitorDescriptor,
    provider: DisplayProvider,
    profile: Option<StoredProfile>,
    selection: ProfileSelection,
    status: SelectionStatus,
    provider_availability: ProviderAvailability,
    generation: u64,
}

impl DisplayProfileSnapshot {
    #[must_use]
    pub const fn monitor(&self) -> MonitorId {
        self.monitor
    }

    #[must_use]
    pub const fn descriptor(&self) -> &MonitorDescriptor {
        &self.descriptor
    }

    #[must_use]
    pub const fn provider(&self) -> DisplayProvider {
        self.provider
    }

    #[must_use]
    pub const fn profile(&self) -> Option<&StoredProfile> {
        self.profile.as_ref()
    }

    #[must_use]
    pub fn profile_id(&self) -> Option<DisplayProfileId> {
        self.profile.as_ref().map(StoredProfile::id)
    }

    #[must_use]
    pub const fn selection(&self) -> ProfileSelection {
        self.selection
    }

    #[must_use]
    pub const fn status(&self) -> SelectionStatus {
        self.status
    }

    #[must_use]
    pub const fn provider_availability(&self) -> ProviderAvailability {
        self.provider_availability
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub fn immutable_profile_bytes(&self) -> Option<Arc<[u8]>> {
        self.profile.as_ref().map(StoredProfile::bytes)
    }

    #[must_use]
    pub fn hdr_capability(&self) -> HdrCapability {
        let hdr = self.descriptor.hdr();
        HdrCapability {
            supported: hdr.supported,
            active: hdr.active,
        }
    }
}

#[derive(Debug, Clone)]
pub struct UserProfile {
    profile: StoredProfile,
    label: String,
}

impl UserProfile {
    #[must_use]
    pub const fn profile(&self) -> &StoredProfile {
        &self.profile
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }
}

pub type FallbackProfile = UserProfile;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayProfileReceipt {
    generation: u64,
    monitor_count: usize,
    changed_monitor_count: usize,
}

impl DisplayProfileReceipt {
    #[must_use]
    pub const fn generation(self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn monitor_count(self) -> usize {
        self.monitor_count
    }

    #[must_use]
    pub const fn changed_monitor_count(self) -> usize {
        self.changed_monitor_count
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceError {
    InvalidLabel,
    Profile(crate::IccProfileError),
    MonitorNotFound(MonitorId),
}

impl fmt::Display for ServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLabel => {
                formatter.write_str("profile label is empty, oversized, or unsafe")
            }
            Self::Profile(error) => error.fmt(formatter),
            Self::MonitorNotFound(monitor) => write!(formatter, "monitor {monitor} is not active"),
        }
    }
}

impl std::error::Error for ServiceError {}

impl From<crate::IccProfileError> for ServiceError {
    fn from(value: crate::IccProfileError) -> Self {
        Self::Profile(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotRequestError {
    MonitorNotFound,
    Removed,
}

impl fmt::Display for SnapshotRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MonitorNotFound => "monitor has no discovered snapshot",
            Self::Removed => "monitor was removed and cannot provide a new snapshot",
        })
    }
}

impl std::error::Error for SnapshotRequestError {}

#[derive(Debug, Default)]
pub struct DisplayProfileService {
    store: ManagedProfileStore,
    snapshots: BTreeMap<MonitorId, DisplayProfileSnapshot>,
    inventory: BTreeMap<MonitorId, ProviderMonitor>,
    removed: BTreeSet<MonitorId>,
    overrides: BTreeMap<MonitorId, UserProfile>,
    fallback: Option<FallbackProfile>,
    events: EventQueue,
    generation: u64,
}

impl DisplayProfileService {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reconciles a bounded provider inventory atomically and publishes only changed state.
    ///
    /// # Errors
    ///
    /// Returns an error only when a provider descriptor is internally inconsistent. Profile
    /// acquisition failures are represented in the snapshot's typed degraded/stale status.
    pub fn reconcile(
        &mut self,
        monitors: impl IntoIterator<Item = ProviderMonitor>,
    ) -> Result<DisplayProfileReceipt, ServiceError> {
        let incoming = monitors
            .into_iter()
            .map(|monitor| (monitor.descriptor().id(), monitor))
            .collect::<BTreeMap<_, _>>();
        let removed = self
            .inventory
            .keys()
            .filter(|id| !incoming.contains_key(id))
            .copied()
            .collect::<Vec<_>>();
        let mut changed = !removed.is_empty();
        for (id, monitor) in &incoming {
            changed |= self.inventory.get(id).is_none_or(|old| {
                old.descriptor() != monitor.descriptor()
                    || old.provider() != monitor.provider()
                    || old.probe() != monitor.probe()
                    || old.availability() != monitor.availability()
            });
        }
        if changed {
            self.generation = self.generation.saturating_add(1);
        }
        for monitor in removed {
            self.snapshots.remove(&monitor);
            self.removed.insert(monitor);
            self.events.push(DisplayProfileEvent::MonitorRemoved {
                monitor,
                generation: self.generation,
            });
        }

        let previous = self.snapshots.clone();
        let mut changed_monitor_count = removed_count(&previous, &incoming);
        let mut next_snapshots = BTreeMap::new();
        for (id, monitor) in &incoming {
            self.removed.remove(id);
            let next = self.select_snapshot(monitor, previous.get(id));
            let old = previous.get(id);
            let is_changed = old.is_none_or(|old| {
                old.profile_id() != next.profile_id()
                    || old.selection() != next.selection()
                    || old.status() != next.status()
                    || old.descriptor() != next.descriptor()
                    || old.provider() != next.provider()
                    || old.provider_availability() != next.provider_availability()
            });
            if is_changed {
                changed_monitor_count += 1;
                self.publish_snapshot_events(old, &next);
            }
            next_snapshots.insert(*id, next);
        }
        let monitor_count = next_snapshots.len();
        self.snapshots = next_snapshots;
        self.inventory = incoming;
        Ok(DisplayProfileReceipt {
            generation: self.generation,
            monitor_count,
            changed_monitor_count,
        })
    }

    /// Returns the immutable snapshot for one active monitor.
    ///
    /// # Errors
    ///
    /// Returns `Removed` for a monitor that was hot-unplugged and `MonitorNotFound` for an
    /// identity that has never been discovered.
    pub fn snapshot(
        &self,
        monitor: MonitorId,
    ) -> Result<DisplayProfileSnapshot, SnapshotRequestError> {
        self.snapshots.get(&monitor).cloned().ok_or_else(|| {
            if self.removed.contains(&monitor) {
                SnapshotRequestError::Removed
            } else {
                SnapshotRequestError::MonitorNotFound
            }
        })
    }

    pub fn snapshots(&self) -> impl Iterator<Item = &DisplayProfileSnapshot> {
        self.snapshots.values()
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub fn events(&mut self) -> Vec<DisplayProfileEvent> {
        self.events.drain()
    }

    #[must_use]
    pub const fn profile_store(&self) -> &ManagedProfileStore {
        &self.store
    }

    /// Installs a validated per-monitor override in the managed immutable profile store.
    ///
    /// # Errors
    ///
    /// Returns an error when the monitor is not active, the label is invalid, or the profile bytes
    /// fail ICC validation.
    pub fn set_override(
        &mut self,
        monitor: MonitorId,
        label: impl Into<String>,
        bytes: &[u8],
    ) -> Result<DisplayProfileReceipt, ServiceError> {
        self.ensure_monitor(monitor)?;
        let label = checked_label(label.into())?;
        let profile = self.store.insert(bytes)?;
        self.overrides
            .insert(monitor, UserProfile { profile, label });
        self.generation = self.generation.saturating_add(1);
        self.events.push(DisplayProfileEvent::OverrideChanged {
            monitor,
            generation: self.generation,
        });
        self.reconcile_current()
    }

    /// Removes a per-monitor override and atomically re-evaluates provider precedence.
    ///
    /// # Errors
    ///
    /// Returns an error when the monitor is not active or the current inventory cannot be
    /// reconciled.
    pub fn remove_override(
        &mut self,
        monitor: MonitorId,
    ) -> Result<DisplayProfileReceipt, ServiceError> {
        self.ensure_monitor(monitor)?;
        self.overrides.remove(&monitor);
        self.generation = self.generation.saturating_add(1);
        self.events.push(DisplayProfileEvent::OverrideChanged {
            monitor,
            generation: self.generation,
        });
        self.reconcile_current()
    }

    /// Selects a validated profile for monitors without an operating-system profile.
    ///
    /// # Errors
    ///
    /// Returns an error when the label is invalid or the profile bytes fail ICC validation.
    pub fn set_fallback(
        &mut self,
        label: impl Into<String>,
        bytes: &[u8],
    ) -> Result<DisplayProfileReceipt, ServiceError> {
        let label = checked_label(label.into())?;
        let profile = self.store.insert(bytes)?;
        self.fallback = Some(UserProfile { profile, label });
        self.generation = self.generation.saturating_add(1);
        self.events.push(DisplayProfileEvent::FallbackChanged {
            generation: self.generation,
        });
        self.reconcile_current()
    }

    /// Removes the explicit fallback and re-evaluates every active monitor.
    ///
    /// # Errors
    ///
    /// Returns an error if the current inventory cannot be reconciled.
    pub fn clear_fallback(&mut self) -> Result<DisplayProfileReceipt, ServiceError> {
        self.fallback = None;
        self.generation = self.generation.saturating_add(1);
        self.events.push(DisplayProfileEvent::FallbackChanged {
            generation: self.generation,
        });
        self.reconcile_current()
    }

    fn select_snapshot(
        &mut self,
        monitor: &ProviderMonitor,
        old: Option<&DisplayProfileSnapshot>,
    ) -> DisplayProfileSnapshot {
        let id = monitor.descriptor().id();
        let mut status = SelectionStatus::Active;
        let selected = if let Some(override_profile) = self.overrides.get(&id) {
            Some((override_profile.profile.clone(), ProfileSelection::Override))
        } else {
            match monitor.probe() {
                ProfileProbe::Current(bytes) => match self.store.insert(bytes) {
                    Ok(profile) => Some((profile, ProfileSelection::OperatingSystem)),
                    Err(error) => old.and_then(|snapshot| {
                        status = SelectionStatus::Stale(stale_reason(error));
                        snapshot
                            .profile()
                            .cloned()
                            .map(|profile| (profile, ProfileSelection::OperatingSystem))
                    }),
                },
                ProfileProbe::Failed(reason) => old.and_then(|snapshot| {
                    status = SelectionStatus::Stale(stale_reason_from_probe(reason));
                    snapshot
                        .profile()
                        .cloned()
                        .map(|profile| (profile, ProfileSelection::OperatingSystem))
                }),
                ProfileProbe::Absent | ProfileProbe::Unavailable => self
                    .fallback
                    .clone()
                    .map(|profile| (profile.profile, ProfileSelection::UserFallback)),
            }
        };
        let (profile, selection) = selected.map_or_else(
            || {
                status = SelectionStatus::Degraded(match monitor.probe() {
                    ProfileProbe::Unavailable => DegradedReason::ProviderUnavailable,
                    ProfileProbe::Absent => DegradedReason::ProfileAbsent,
                    ProfileProbe::Failed(ProfileProbeFailure::PermissionDenied) => {
                        DegradedReason::PermissionDenied
                    }
                    ProfileProbe::Failed(_) | ProfileProbe::Current(_) => {
                        DegradedReason::ProfileUnreadable
                    }
                });
                (None, ProfileSelection::Unprofiled)
            },
            |(profile, selection)| (Some(profile), selection),
        );
        DisplayProfileSnapshot {
            monitor: id,
            descriptor: monitor.descriptor().clone(),
            provider: monitor.provider(),
            profile,
            selection,
            status,
            provider_availability: monitor.availability(),
            generation: self.generation,
        }
    }

    fn publish_snapshot_events(
        &mut self,
        old: Option<&DisplayProfileSnapshot>,
        next: &DisplayProfileSnapshot,
    ) {
        let monitor = next.monitor();
        if old.is_none() {
            self.events.push(DisplayProfileEvent::MonitorAdded {
                monitor,
                generation: self.generation,
            });
        } else if old.is_some_and(|old| old.descriptor() != next.descriptor()) {
            self.events.push(DisplayProfileEvent::MonitorChanged {
                monitor,
                generation: self.generation,
            });
        }
        if old.is_some_and(|old| old.profile_id() != next.profile_id()) {
            self.events.push(DisplayProfileEvent::ProfileHashChanged {
                monitor,
                previous: old.and_then(DisplayProfileSnapshot::profile_id),
                current: next.profile_id(),
                generation: self.generation,
            });
        }
        if old.is_some_and(|old| old.provider_availability() != next.provider_availability()) {
            self.events
                .push(DisplayProfileEvent::ProviderAvailabilityChanged {
                    monitor,
                    provider: next.provider(),
                    available: next.provider_availability() == ProviderAvailability::Available,
                    generation: self.generation,
                });
        }
        if old
            .is_none_or(|old| old.selection() != next.selection() || old.status() != next.status())
        {
            self.events.push(DisplayProfileEvent::SelectionChanged {
                monitor,
                selection: next.selection(),
                status: next.status(),
                generation: self.generation,
            });
        }
    }

    fn ensure_monitor(&self, monitor: MonitorId) -> Result<(), ServiceError> {
        self.inventory
            .contains_key(&monitor)
            .then_some(())
            .ok_or(ServiceError::MonitorNotFound(monitor))
    }

    fn reconcile_current(&mut self) -> Result<DisplayProfileReceipt, ServiceError> {
        let inventory = self.inventory.values().cloned().collect::<Vec<_>>();
        self.reconcile(inventory)
    }
}

#[derive(Debug, Clone, Default)]
pub struct WindowPresentation {
    windows: BTreeMap<String, WindowMonitorState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WindowMonitorState {
    monitor: MonitorId,
    generation: u64,
}

impl WindowPresentation {
    /// Changes a window's target monitor without changing any other window's state.
    pub fn move_window(&mut self, window: impl Into<String>, snapshot: &DisplayProfileSnapshot) {
        self.windows.insert(
            window.into(),
            WindowMonitorState {
                monitor: snapshot.monitor(),
                generation: snapshot.generation(),
            },
        );
    }

    /// Accepts only a complete transform built for the window's current monitor/generation.
    #[must_use]
    pub fn accepts_transform(&self, window: &str, monitor: MonitorId, generation: u64) -> bool {
        self.windows
            .get(window)
            .is_some_and(|state| state.monitor == monitor && state.generation == generation)
    }
}

fn checked_label(label: String) -> Result<String, ServiceError> {
    if label.is_empty() || label.len() > 256 || label.chars().any(char::is_control) {
        Err(ServiceError::InvalidLabel)
    } else {
        Ok(label)
    }
}

fn stale_reason(error: crate::IccProfileError) -> StaleReason {
    match error {
        crate::IccProfileError::Oversized => StaleReason::ProfileOversized,
        crate::IccProfileError::UnsupportedDeviceLink
        | crate::IccProfileError::UnsupportedColorSpace => StaleReason::ProfileUnsupported,
        _ => StaleReason::ProfileInvalid,
    }
}

fn stale_reason_from_probe(reason: &ProfileProbeFailure) -> StaleReason {
    match reason {
        ProfileProbeFailure::ChangedDuringRead => StaleReason::ProfileChangedDuringRead,
        ProfileProbeFailure::Unsupported => StaleReason::ProfileUnsupported,
        ProfileProbeFailure::Oversized => StaleReason::ProfileOversized,
        ProfileProbeFailure::Invalid => StaleReason::ProfileInvalid,
        ProfileProbeFailure::PermissionDenied | ProfileProbeFailure::Unreadable => {
            StaleReason::ProfileReadFailed
        }
    }
}

fn removed_count(
    previous: &BTreeMap<MonitorId, DisplayProfileSnapshot>,
    incoming: &BTreeMap<MonitorId, ProviderMonitor>,
) -> usize {
    previous
        .keys()
        .filter(|id| !incoming.contains_key(id))
        .count()
}
