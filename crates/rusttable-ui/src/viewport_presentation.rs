//! Generation-safe GTK viewport presentation state.
//!
//! The render/GPU application service supplies a validated RGBA8 frame through
//! [`DisplayPresentationPort`].  GTK only owns the paintable and visible status; it never owns
//! color transforms, monitor discovery, export pixels, or device resources.

#![allow(clippy::missing_errors_doc)]

use rusttable_core::{PhotoId, Revision};
use rusttable_display_profile::{MonitorId, ProfileSelection};

use crate::presentation::{PreviewDimensions, Rgba8PreviewMetadata};
pub use crate::viewport_navigation::NavigationCrop;

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

/// Monotonic identity for a selected-photo viewport projection.
///
/// The GTK controls attach this value to every command. An orchestrator can advance it when a
/// new selected-photo/edit projection is published and discard commands from an older view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ViewportGeneration(u64);

impl ViewportGeneration {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Darktable's navigation-box zoom choices, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DarkroomZoom {
    Small,
    #[default]
    Fit,
    Fill,
    FiftyPercent,
    OneHundredPercent,
    TwoHundredPercent,
    FourHundredPercent,
    EightHundredPercent,
    SixteenHundredPercent,
}

impl DarkroomZoom {
    pub const ALL: [Self; 9] = [
        Self::Small,
        Self::Fit,
        Self::Fill,
        Self::FiftyPercent,
        Self::OneHundredPercent,
        Self::TwoHundredPercent,
        Self::FourHundredPercent,
        Self::EightHundredPercent,
        Self::SixteenHundredPercent,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Fit => "fit",
            Self::Fill => "fill",
            Self::FiftyPercent => "50%",
            Self::OneHundredPercent => "100%",
            Self::TwoHundredPercent => "200%",
            Self::FourHundredPercent => "400%",
            Self::EightHundredPercent => "800%",
            Self::SixteenHundredPercent => "1600%",
        }
    }

    #[must_use]
    pub const fn percent(self) -> Option<u16> {
        match self {
            Self::Small | Self::Fit | Self::Fill => None,
            Self::FiftyPercent => Some(50),
            Self::OneHundredPercent => Some(100),
            Self::TwoHundredPercent => Some(200),
            Self::FourHundredPercent => Some(400),
            Self::EightHundredPercent => Some(800),
            Self::SixteenHundredPercent => Some(1_600),
        }
    }

    #[must_use]
    pub const fn index(self) -> u32 {
        match self {
            Self::Small => 0,
            Self::Fit => 1,
            Self::Fill => 2,
            Self::FiftyPercent => 3,
            Self::OneHundredPercent => 4,
            Self::TwoHundredPercent => 5,
            Self::FourHundredPercent => 6,
            Self::EightHundredPercent => 7,
            Self::SixteenHundredPercent => 8,
        }
    }

    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Small => Self::Fit,
            Self::Fit => Self::Fill,
            Self::Fill => Self::FiftyPercent,
            Self::FiftyPercent => Self::OneHundredPercent,
            Self::OneHundredPercent => Self::TwoHundredPercent,
            Self::TwoHundredPercent => Self::FourHundredPercent,
            Self::FourHundredPercent => Self::EightHundredPercent,
            Self::EightHundredPercent | Self::SixteenHundredPercent => Self::SixteenHundredPercent,
        }
    }

    #[must_use]
    pub const fn previous(self) -> Self {
        match self {
            Self::Small | Self::Fit => Self::Small,
            Self::Fill => Self::Fit,
            Self::FiftyPercent => Self::Fill,
            Self::OneHundredPercent => Self::FiftyPercent,
            Self::TwoHundredPercent => Self::OneHundredPercent,
            Self::FourHundredPercent => Self::TwoHundredPercent,
            Self::EightHundredPercent => Self::FourHundredPercent,
            Self::SixteenHundredPercent => Self::EightHundredPercent,
        }
    }
}

/// Normalized pan offset used by the viewport projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ViewportPan {
    x: i16,
    y: i16,
}

impl ViewportPan {
    #[must_use]
    pub fn new(x: i16, y: i16) -> Self {
        Self {
            x: clamp_pan(i32::from(x)),
            y: clamp_pan(i32::from(y)),
        }
    }

    #[must_use]
    pub const fn x(self) -> i16 {
        self.x
    }

    #[must_use]
    pub const fn y(self) -> i16 {
        self.y
    }

    #[must_use]
    pub fn adjust(self, delta_x: i32, delta_y: i32) -> Self {
        Self::new(
            clamp_pan(i32::from(self.x) + delta_x),
            clamp_pan(i32::from(self.y) + delta_y),
        )
    }
}

fn clamp_pan(value: i32) -> i16 {
    let value = value.clamp(-1_000, 1_000);
    i16::try_from(value).unwrap_or(0)
}

/// Before/after projection shown by the darkroom viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ViewportComparison {
    #[default]
    Edited,
    Before,
}

impl ViewportComparison {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Edited => "edited",
            Self::Before => "before",
        }
    }
}

/// Mutually exclusive Darktable soft-proof and gamut-check projection modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ViewportColorMode {
    #[default]
    Normal,
    SoftProof,
    GamutCheck,
}

impl ViewportColorMode {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::SoftProof => "soft proof",
            Self::GamutCheck => "gamut check",
        }
    }
}

/// Typed user intent emitted by the darkroom toolbar and navigation gestures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DarkroomViewportAction {
    SetZoom(DarkroomZoom),
    ZoomIn,
    ZoomOut,
    Fit,
    Pan { delta_x: i32, delta_y: i32 },
    ToggleBeforeAfter,
    ToggleSoftProof,
    ToggleGamutCheck,
}

/// A viewport action tagged with the selected-photo generation that produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DarkroomViewportCommand {
    generation: ViewportGeneration,
    action: DarkroomViewportAction,
}

impl DarkroomViewportCommand {
    #[must_use]
    pub const fn new(generation: ViewportGeneration, action: DarkroomViewportAction) -> Self {
        Self { generation, action }
    }

    #[must_use]
    pub const fn generation(self) -> ViewportGeneration {
        self.generation
    }

    #[must_use]
    pub const fn action(self) -> DarkroomViewportAction {
        self.action
    }
}

/// Revision-safe state for the selected darkroom photo and its viewport projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DarkroomViewportState {
    photo_id: Option<PhotoId>,
    edit_revision: Option<Revision>,
    generation: ViewportGeneration,
    zoom: DarkroomZoom,
    pan: ViewportPan,
    comparison: ViewportComparison,
    color_mode: ViewportColorMode,
}

impl DarkroomViewportState {
    #[must_use]
    pub const fn photo_id(self) -> Option<PhotoId> {
        self.photo_id
    }

    #[must_use]
    pub const fn edit_revision(self) -> Option<Revision> {
        self.edit_revision
    }

    #[must_use]
    pub const fn generation(self) -> ViewportGeneration {
        self.generation
    }

    #[must_use]
    pub const fn zoom(self) -> DarkroomZoom {
        self.zoom
    }

    #[must_use]
    pub const fn pan(self) -> ViewportPan {
        self.pan
    }

    #[must_use]
    pub const fn comparison(self) -> ViewportComparison {
        self.comparison
    }

    #[must_use]
    pub const fn color_mode(self) -> ViewportColorMode {
        self.color_mode
    }

    /// Starts a new selected-photo projection and resets transient viewport state.
    pub fn select(
        &mut self,
        photo_id: PhotoId,
        edit_revision: Revision,
        generation: ViewportGeneration,
    ) {
        self.photo_id = Some(photo_id);
        self.edit_revision = Some(edit_revision);
        self.generation = generation;
        self.reset_view();
    }

    /// Reconciles the edit revision of a completed render without resetting viewport controls.
    pub fn set_edit_revision(
        &mut self,
        edit_revision: Revision,
        generation: ViewportGeneration,
    ) -> bool {
        if self.generation != generation || self.photo_id.is_none() {
            return false;
        }
        if self.edit_revision == Some(edit_revision) {
            return false;
        }
        self.edit_revision = Some(edit_revision);
        true
    }

    /// Clears selection while leaving a truthful empty/no-photo viewport.
    pub fn clear_selection(&mut self) {
        self.photo_id = None;
        self.edit_revision = None;
        self.reset_view();
    }

    /// Applies a command only when it belongs to the current selected-photo generation.
    pub fn apply(&mut self, command: DarkroomViewportCommand) -> bool {
        if command.generation != self.generation {
            return false;
        }
        match command.action {
            DarkroomViewportAction::SetZoom(zoom) => self.set_zoom(zoom),
            DarkroomViewportAction::ZoomIn => self.set_zoom(self.zoom.next()),
            DarkroomViewportAction::ZoomOut => self.set_zoom(self.zoom.previous()),
            DarkroomViewportAction::Fit => self.set_zoom(DarkroomZoom::Fit),
            DarkroomViewportAction::Pan { delta_x, delta_y } => {
                if matches!(self.zoom, DarkroomZoom::Small | DarkroomZoom::Fit) {
                    return false;
                }
                let pan = self.pan.adjust(delta_x, delta_y);
                if pan == self.pan {
                    false
                } else {
                    self.pan = pan;
                    true
                }
            }
            DarkroomViewportAction::ToggleBeforeAfter => {
                self.comparison = match self.comparison {
                    ViewportComparison::Edited => ViewportComparison::Before,
                    ViewportComparison::Before => ViewportComparison::Edited,
                };
                true
            }
            DarkroomViewportAction::ToggleSoftProof => {
                self.color_mode = if self.color_mode == ViewportColorMode::SoftProof {
                    ViewportColorMode::Normal
                } else {
                    ViewportColorMode::SoftProof
                };
                true
            }
            DarkroomViewportAction::ToggleGamutCheck => {
                self.color_mode = if self.color_mode == ViewportColorMode::GamutCheck {
                    ViewportColorMode::Normal
                } else {
                    ViewportColorMode::GamutCheck
                };
                true
            }
        }
    }

    #[must_use]
    pub fn projection_label(self) -> String {
        let Some(_) = self.photo_id else {
            return "no photo selected".to_owned();
        };
        let pan = if self.pan == ViewportPan::default() {
            "centered"
        } else {
            "panned"
        };
        format!(
            "{} · {} · {} · {pan}",
            self.zoom.label(),
            self.comparison.label(),
            self.color_mode.label()
        )
    }

    /// Returns whether a comparison or color-analysis overlay is active.
    #[must_use]
    pub const fn has_active_overlay(self) -> bool {
        matches!(self.comparison, ViewportComparison::Before)
            || !matches!(self.color_mode, ViewportColorMode::Normal)
    }

    fn set_zoom(&mut self, zoom: DarkroomZoom) -> bool {
        if self.zoom == zoom {
            if matches!(zoom, DarkroomZoom::Small | DarkroomZoom::Fit)
                && self.pan != ViewportPan::default()
            {
                self.pan = ViewportPan::default();
                return true;
            }
            return false;
        }
        self.zoom = zoom;
        if matches!(zoom, DarkroomZoom::Small | DarkroomZoom::Fit) {
            self.pan = ViewportPan::default();
        }
        true
    }

    fn reset_view(&mut self) {
        self.zoom = DarkroomZoom::Fit;
        self.pan = ViewportPan::default();
        self.comparison = ViewportComparison::Edited;
        self.color_mode = ViewportColorMode::Normal;
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

    #[test]
    fn darkroom_viewport_actions_are_bounded_and_mutually_exclusive() {
        let mut state = DarkroomViewportState::default();
        let generation = ViewportGeneration::new(7);
        state.select(
            PhotoId::new(42).expect("photo"),
            Revision::from_u64(3),
            generation,
        );

        assert!(state.apply(DarkroomViewportCommand::new(
            generation,
            DarkroomViewportAction::SetZoom(DarkroomZoom::OneHundredPercent),
        )));
        assert!(state.apply(DarkroomViewportCommand::new(
            generation,
            DarkroomViewportAction::Pan {
                delta_x: 9_999,
                delta_y: -9_999,
            },
        )));
        assert_eq!(state.pan(), ViewportPan::new(1_000, -1_000));

        assert!(state.apply(DarkroomViewportCommand::new(
            generation,
            DarkroomViewportAction::ToggleSoftProof,
        )));
        assert_eq!(state.color_mode(), ViewportColorMode::SoftProof);
        assert!(state.apply(DarkroomViewportCommand::new(
            generation,
            DarkroomViewportAction::ToggleGamutCheck,
        )));
        assert_eq!(state.color_mode(), ViewportColorMode::GamutCheck);
        assert_eq!(state.comparison(), ViewportComparison::Edited);
    }

    #[test]
    fn stale_darkroom_viewport_commands_cannot_mutate_new_selection() {
        let mut state = DarkroomViewportState::default();
        state.select(
            PhotoId::new(42).expect("photo"),
            Revision::from_u64(3),
            ViewportGeneration::new(8),
        );
        assert!(!state.apply(DarkroomViewportCommand::new(
            ViewportGeneration::new(7),
            DarkroomViewportAction::ToggleBeforeAfter,
        )));
        assert_eq!(state.comparison(), ViewportComparison::Edited);
        assert_eq!(state.projection_label(), "fit · edited · normal · centered");
        assert!(!state.has_active_overlay());
        assert_eq!(state.navigation_crop().width_milli(), 1_000);
    }

    #[test]
    fn navigation_crop_tracks_active_zoom_and_pan() {
        let mut state = DarkroomViewportState::default();
        let generation = ViewportGeneration::new(9);
        state.select(PhotoId::new(42).expect("photo"), Revision::ZERO, generation);
        assert!(state.apply(DarkroomViewportCommand::new(
            generation,
            DarkroomViewportAction::SetZoom(DarkroomZoom::TwoHundredPercent),
        )));
        assert!(state.apply(DarkroomViewportCommand::new(
            generation,
            DarkroomViewportAction::Pan {
                delta_x: 1_000,
                delta_y: -1_000,
            },
        )));
        let crop = state.navigation_crop();
        assert_eq!(crop.width_milli(), 500);
        assert_eq!(crop.height_milli(), 500);
        assert_eq!(crop.x_milli(), 500);
        assert_eq!(crop.y_milli(), 0);
    }

    #[test]
    fn fit_and_small_reject_pan_like_darktable_zoom_move() {
        let mut state = DarkroomViewportState::default();
        let generation = ViewportGeneration::new(10);
        state.select(PhotoId::new(42).expect("photo"), Revision::ZERO, generation);

        assert!(!state.apply(DarkroomViewportCommand::new(
            generation,
            DarkroomViewportAction::Pan {
                delta_x: 100,
                delta_y: 100,
            },
        )));
        assert_eq!(state.pan(), ViewportPan::default());

        assert!(state.apply(DarkroomViewportCommand::new(
            generation,
            DarkroomViewportAction::SetZoom(DarkroomZoom::Small),
        )));
        assert!(!state.apply(DarkroomViewportCommand::new(
            generation,
            DarkroomViewportAction::Pan {
                delta_x: 100,
                delta_y: 100,
            },
        )));
        assert_eq!(state.pan(), ViewportPan::default());
    }
}
