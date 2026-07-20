use crate::{HdrDescriptor, MonitorDescriptor, MonitorGeometry};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DisplayProvider {
    Colord,
    X11,
    Wayland,
    ColorSync,
    WindowsWcs,
    Synthetic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderAvailability {
    Available,
    Unavailable,
}

#[derive(Debug, Clone)]
pub struct ProviderMonitor {
    descriptor: MonitorDescriptor,
    provider: DisplayProvider,
    probe: ProfileProbe,
    availability: ProviderAvailability,
}

impl ProviderMonitor {
    #[must_use]
    pub fn new(
        descriptor: MonitorDescriptor,
        provider: DisplayProvider,
        probe: ProfileProbe,
    ) -> Self {
        Self {
            descriptor,
            provider,
            probe,
            availability: ProviderAvailability::Available,
        }
    }

    #[must_use]
    pub fn unavailable(descriptor: MonitorDescriptor, provider: DisplayProvider) -> Self {
        Self {
            descriptor,
            provider,
            probe: ProfileProbe::Unavailable,
            availability: ProviderAvailability::Unavailable,
        }
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
    pub const fn probe(&self) -> &ProfileProbe {
        &self.probe
    }

    #[must_use]
    pub const fn availability(&self) -> ProviderAvailability {
        self.availability
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileProbe {
    Current(Vec<u8>),
    Absent,
    Failed(ProfileProbeFailure),
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileProbeFailure {
    PermissionDenied,
    Unreadable,
    ChangedDuringRead,
    Invalid,
    Unsupported,
    Oversized,
}

pub trait PlatformProfileAdapter {
    /// Reads the current platform inventory without exposing native handles or raw labels.
    ///
    /// # Errors
    ///
    /// Returns a typed provider error when the desktop color-management service is unavailable or
    /// returns an invalid monitor inventory.
    fn discover(&mut self) -> Result<Vec<ProviderMonitor>, ProviderError>;
}

/// Safe provider boundary for platform implementations.
///
/// The desktop shell supplies GDK monitor descriptors and platform adapters supply ICC bytes.
/// This fallback adapter makes the provider choice explicit when the relevant desktop service is
/// absent; it never invents an sRGB profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemProfileAdapter {
    provider: DisplayProvider,
}

impl SystemProfileAdapter {
    #[must_use]
    pub const fn new(provider: DisplayProvider) -> Self {
        Self { provider }
    }

    #[must_use]
    pub fn current() -> Self {
        Self::new(current_provider())
    }

    #[must_use]
    pub const fn provider(self) -> DisplayProvider {
        self.provider
    }
}

impl Default for SystemProfileAdapter {
    fn default() -> Self {
        Self::current()
    }
}

impl PlatformProfileAdapter for SystemProfileAdapter {
    fn discover(&mut self) -> Result<Vec<ProviderMonitor>, ProviderError> {
        Err(ProviderError::Unavailable {
            provider: self.provider,
            reason: ProviderUnavailableReason::DesktopServiceNotConnected,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    Unavailable {
        provider: DisplayProvider,
        reason: ProviderUnavailableReason,
    },
    InvalidInventory(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderUnavailableReason {
    DesktopServiceNotConnected,
    UnsupportedWindowSystem,
    PermissionDenied,
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable { provider, reason } => {
                write!(formatter, "{provider:?} provider unavailable: {reason:?}")
            }
            Self::InvalidInventory(reason) => {
                write!(formatter, "invalid monitor inventory: {reason}")
            }
        }
    }
}

impl std::error::Error for ProviderError {}

#[cfg(target_os = "linux")]
fn current_provider() -> DisplayProvider {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        DisplayProvider::Wayland
    } else if std::env::var_os("DISPLAY").is_some() {
        DisplayProvider::X11
    } else {
        DisplayProvider::Colord
    }
}

#[cfg(target_os = "macos")]
fn current_provider() -> DisplayProvider {
    DisplayProvider::ColorSync
}

#[cfg(target_os = "windows")]
fn current_provider() -> DisplayProvider {
    DisplayProvider::WindowsWcs
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn current_provider() -> DisplayProvider {
    DisplayProvider::Synthetic
}

/// Creates the descriptor used by the GTK adapter without retaining native display strings.
///
/// # Errors
///
/// Returns an error when the supplied geometry or UI-local alias is invalid.
#[allow(clippy::too_many_arguments)]
pub fn descriptor_from_gdk_evidence(
    platform: &str,
    connector: Option<&str>,
    manufacturer: Option<&str>,
    model: Option<&str>,
    edid: Option<&[u8]>,
    alias: impl Into<String>,
    geometry: (i32, i32, u32, u32, u32),
    hdr: HdrDescriptor,
) -> Result<MonitorDescriptor, crate::MonitorIdError> {
    let id = crate::MonitorId::from_platform_parts(platform, connector, manufacturer, model, edid);
    MonitorDescriptor::new(
        id,
        alias,
        MonitorGeometry::new(geometry.0, geometry.1, geometry.2, geometry.3, geometry.4)?,
        hdr,
    )
}
