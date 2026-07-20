//! Generation-safe GTK viewport presentation state.
//!
//! The render/GPU application service supplies a validated RGBA8 frame through
//! [`DisplayPresentationPort`].  GTK only owns the paintable and visible status; it never owns
//! color transforms, monitor discovery, export pixels, or device resources.

#![allow(clippy::missing_errors_doc)]

use rusttable_core::{PhotoId, Revision};
use rusttable_display_profile::{MonitorId, ProfileSelection};

use crate::presentation::{PreviewDimensions, Rgba8PreviewMetadata};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PresentationGeneration(u64);

impl PresentationGeneration {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentationMode {
    Sdr,
    Hdr,
}

impl PresentationMode {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Sdr => "SDR",
            Self::Hdr => "HDR",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SdrFallbackReason {
    MissingMonitorProfile,
    UnsupportedHdr,
    SurfaceUnavailable,
    DeviceUnavailable,
    TransformUnavailable,
}

impl SdrFallbackReason {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::MissingMonitorProfile => "monitor profile unavailable",
            Self::UnsupportedHdr => "HDR surface unsupported",
            Self::SurfaceUnavailable => "display surface unavailable",
            Self::DeviceUnavailable => "presentation device unavailable",
            Self::TransformUnavailable => "exact transform unavailable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentationFailure {
    InvalidFrame,
    StaleGeneration,
    StalePhoto,
    ServiceUnavailable,
}

impl PresentationFailure {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::InvalidFrame => "presentation frame is invalid",
            Self::StaleGeneration => "presentation result is stale",
            Self::StalePhoto => "presentation result belongs to another photo",
            Self::ServiceUnavailable => "color-managed presentation unavailable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentationStatus {
    Rendering,
    Ready {
        mode: PresentationMode,
        profile: ProfileSelection,
    },
    SrgbFallback(SdrFallbackReason),
    Failed(PresentationFailure),
}

impl PresentationStatus {
    #[must_use]
    pub fn label(self) -> String {
        match self {
            Self::Rendering => "Color-managed presentation: rendering".to_owned(),
            Self::Ready { mode, profile } => format!(
                "Color-managed presentation: {} · {}",
                mode.label(),
                profile_label(profile)
            ),
            Self::SrgbFallback(reason) => format!(
                "Color-managed presentation: sRGB fallback · {}",
                reason.label()
            ),
            Self::Failed(reason) => format!("Color-managed presentation: {}", reason.label()),
        }
    }

    #[must_use]
    pub const fn is_explicit_fallback(self) -> bool {
        matches!(self, Self::SrgbFallback(_))
    }
}

fn profile_label(profile: ProfileSelection) -> &'static str {
    match profile {
        ProfileSelection::Override => "override profile",
        ProfileSelection::OperatingSystem => "OS profile",
        ProfileSelection::UserFallback => "user fallback profile",
        ProfileSelection::Unprofiled => "unprofiled",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayPresentationRequest {
    photo_id: PhotoId,
    edit_revision: Revision,
    monitor: Option<MonitorId>,
    profile_generation: PresentationGeneration,
    mode: PresentationMode,
    proof_enabled: bool,
    gamut_warning: bool,
}

impl DisplayPresentationRequest {
    #[must_use]
    pub const fn new(
        photo_id: PhotoId,
        edit_revision: Revision,
        monitor: Option<MonitorId>,
        profile_generation: PresentationGeneration,
        mode: PresentationMode,
    ) -> Self {
        Self {
            photo_id,
            edit_revision,
            monitor,
            profile_generation,
            mode,
            proof_enabled: false,
            gamut_warning: false,
        }
    }

    #[must_use]
    pub const fn photo_id(self) -> PhotoId {
        self.photo_id
    }
    #[must_use]
    pub const fn edit_revision(self) -> Revision {
        self.edit_revision
    }
    #[must_use]
    pub const fn monitor(self) -> Option<MonitorId> {
        self.monitor
    }
    #[must_use]
    pub const fn profile_generation(self) -> PresentationGeneration {
        self.profile_generation
    }
    #[must_use]
    pub const fn mode(self) -> PresentationMode {
        self.mode
    }
    #[must_use]
    pub const fn proof_enabled(self) -> bool {
        self.proof_enabled
    }
    #[must_use]
    pub const fn gamut_warning(self) -> bool {
        self.gamut_warning
    }

    #[must_use]
    pub const fn with_proof(mut self, enabled: bool) -> Self {
        self.proof_enabled = enabled;
        self
    }

    #[must_use]
    pub const fn with_gamut_warning(mut self, enabled: bool) -> Self {
        self.gamut_warning = enabled;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresentationTicket {
    request: DisplayPresentationRequest,
}

impl PresentationTicket {
    #[must_use]
    pub const fn new(request: DisplayPresentationRequest) -> Self {
        Self { request }
    }

    #[must_use]
    pub const fn request(self) -> DisplayPresentationRequest {
        self.request
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayPresentationFrame {
    ticket: PresentationTicket,
    metadata: Rgba8PreviewMetadata,
    status: PresentationStatus,
}

impl DisplayPresentationFrame {
    /// Constructs a frame only from bounded, already-validated RGBA8 presentation bytes.
    ///
    /// # Errors
    ///
    /// Returns [`PresentationFailure::InvalidFrame`] when the payload dimensions do not match.
    pub fn new(
        ticket: PresentationTicket,
        dimensions: PreviewDimensions,
        pixels: Vec<u8>,
        status: PresentationStatus,
    ) -> Result<Self, PresentationFailure> {
        let label = crate::presentation::PresentationText::new(status.label())
            .map_err(|_| PresentationFailure::InvalidFrame)?;
        let metadata = Rgba8PreviewMetadata::new(dimensions, label, pixels)
            .map_err(|_| PresentationFailure::InvalidFrame)?;
        Ok(Self {
            ticket,
            metadata,
            status,
        })
    }

    #[must_use]
    pub const fn ticket(&self) -> PresentationTicket {
        self.ticket
    }
    #[must_use]
    pub const fn metadata(&self) -> &Rgba8PreviewMetadata {
        &self.metadata
    }
    #[must_use]
    pub const fn status(&self) -> PresentationStatus {
        self.status
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum DisplayPresentationState {
    #[default]
    Unavailable,
    Rendering {
        ticket: PresentationTicket,
    },
    Ready(DisplayPresentationFrame),
    SrgbFallback(DisplayPresentationFrame),
    Failed {
        ticket: Option<PresentationTicket>,
        reason: PresentationFailure,
    },
}

impl DisplayPresentationState {
    #[must_use]
    pub fn status(&self) -> PresentationStatus {
        match self {
            Self::Unavailable => {
                PresentationStatus::Failed(PresentationFailure::ServiceUnavailable)
            }
            Self::Rendering { .. } => PresentationStatus::Rendering,
            Self::Ready(frame) | Self::SrgbFallback(frame) => frame.status(),
            Self::Failed { reason, .. } => PresentationStatus::Failed(*reason),
        }
    }
}

/// Service boundary for GPU/CPU color presentation; no GTK or resource handles cross it.
pub trait DisplayPresentationPort {
    fn request_presentation(
        &mut self,
        request: DisplayPresentationRequest,
    ) -> Result<PresentationTicket, PresentationFailure>;
}

#[derive(Debug, Default)]
pub struct DisplayPresentationController {
    state: DisplayPresentationState,
    current: Option<PresentationTicket>,
}

impl DisplayPresentationController {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: DisplayPresentationState::Unavailable,
            current: None,
        }
    }

    #[must_use]
    pub const fn state(&self) -> &DisplayPresentationState {
        &self.state
    }

    pub fn request<P: DisplayPresentationPort>(
        &mut self,
        port: &mut P,
        request: DisplayPresentationRequest,
    ) -> Result<PresentationTicket, PresentationFailure> {
        let ticket = port.request_presentation(request)?;
        self.current = Some(ticket);
        self.state = DisplayPresentationState::Rendering { ticket };
        Ok(ticket)
    }

    /// Publishes only a frame for the current photo/edit/monitor generation.
    pub fn publish(&mut self, frame: DisplayPresentationFrame) -> bool {
        let Some(current) = self.current else {
            self.state = DisplayPresentationState::Failed {
                ticket: None,
                reason: PresentationFailure::StaleGeneration,
            };
            return false;
        };
        if current != frame.ticket {
            self.state = DisplayPresentationState::Failed {
                ticket: Some(current),
                reason: PresentationFailure::StaleGeneration,
            };
            return false;
        }
        self.state = if frame.status.is_explicit_fallback() {
            DisplayPresentationState::SrgbFallback(frame)
        } else {
            DisplayPresentationState::Ready(frame)
        };
        true
    }

    pub fn fail(&mut self, reason: PresentationFailure) {
        self.state = DisplayPresentationState::Failed {
            ticket: self.current,
            reason,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakePort;
    impl DisplayPresentationPort for FakePort {
        fn request_presentation(
            &mut self,
            request: DisplayPresentationRequest,
        ) -> Result<PresentationTicket, PresentationFailure> {
            Ok(PresentationTicket { request })
        }
    }

    fn request(photo: u128, generation: u64) -> DisplayPresentationRequest {
        DisplayPresentationRequest::new(
            PhotoId::new(photo).expect("photo"),
            Revision::from_u64(4),
            None,
            PresentationGeneration::new(generation),
            PresentationMode::Sdr,
        )
    }

    fn frame(ticket: PresentationTicket, status: PresentationStatus) -> DisplayPresentationFrame {
        DisplayPresentationFrame::new(
            ticket,
            PreviewDimensions::new(1, 1).expect("dimensions"),
            vec![0, 0, 0, 255],
            status,
        )
        .expect("frame")
    }

    #[test]
    fn late_generation_cannot_replace_the_current_viewport() {
        let mut controller = DisplayPresentationController::new();
        let mut port = FakePort;
        let old = controller
            .request(&mut port, request(1, 1))
            .expect("old request");
        let current = controller
            .request(&mut port, request(1, 2))
            .expect("current request");
        assert!(!controller.publish(frame(
            old,
            PresentationStatus::Ready {
                mode: PresentationMode::Sdr,
                profile: ProfileSelection::OperatingSystem
            }
        )));
        assert!(matches!(
            controller.state(),
            DisplayPresentationState::Failed {
                reason: PresentationFailure::StaleGeneration,
                ..
            }
        ));
        assert!(controller.publish(frame(
            current,
            PresentationStatus::Ready {
                mode: PresentationMode::Sdr,
                profile: ProfileSelection::OperatingSystem
            }
        )));
    }

    #[test]
    fn unsupported_hdr_is_a_visible_srgb_fallback_not_a_ready_hdr_frame() {
        let mut controller = DisplayPresentationController::new();
        let mut port = FakePort;
        let ticket = controller
            .request(&mut port, request(1, 1))
            .expect("request");
        assert!(controller.publish(frame(
            ticket,
            PresentationStatus::SrgbFallback(SdrFallbackReason::UnsupportedHdr)
        )));
        assert!(matches!(
            controller.state(),
            DisplayPresentationState::SrgbFallback(_)
        ));
        assert!(
            controller
                .state()
                .status()
                .label()
                .contains("sRGB fallback")
        );
    }

    #[test]
    fn status_is_explicit_about_profile_and_mode() {
        let label = PresentationStatus::Ready {
            mode: PresentationMode::Hdr,
            profile: ProfileSelection::Override,
        }
        .label();
        assert_eq!(label, "Color-managed presentation: HDR · override profile");
    }
}
