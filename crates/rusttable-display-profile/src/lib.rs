#![forbid(unsafe_code)]
#![doc = "Safe monitor identity, display-profile acquisition, and change tracking."]

mod events;
mod identity;
mod profile;
mod provider;
mod service;

pub use events::{DisplayProfileEvent, EventQueue, MAX_QUEUED_EVENTS};
pub use identity::{HdrDescriptor, MonitorDescriptor, MonitorGeometry, MonitorId, MonitorIdError};
pub use profile::{
    DisplayProfileId, IccProfileError, MAX_PROFILE_BYTES, MIN_PROFILE_BYTES, ManagedProfileStore,
    ProfileMetadata, ProfileTransformError, StoredProfile,
};
pub use provider::{
    DisplayProvider, PlatformProfileAdapter, ProfileProbe, ProfileProbeFailure,
    ProviderAvailability, ProviderError, ProviderMonitor, SystemProfileAdapter,
    descriptor_from_gdk_evidence,
};
pub use service::{
    DegradedReason, DisplayProfileReceipt, DisplayProfileService, DisplayProfileSnapshot,
    FallbackProfile, HdrCapability, ProfileSelection, ProfileSource, SelectionStatus, ServiceError,
    SnapshotRequestError, StaleReason, UserProfile, WindowPresentation,
};
