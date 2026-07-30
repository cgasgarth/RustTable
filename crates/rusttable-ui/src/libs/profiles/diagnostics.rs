//! Darktable-aligned, privacy-safe projection of display-profile decisions.
//!
//! This module consumes the display-profile service's immutable decision and receipt types. It
//! does not inspect profile bytes or initiate color transforms; it only makes the decision state
//! visible to the GTK shell.

use gtk4::accessible::Property;
use gtk4::prelude::*;
use rusttable_display_profile::{
    DegradedReason, DisplayProfileReceipt, DisplayProfileSnapshot, HdrCapability, ProfileSelection,
    SelectionStatus, StaleReason,
};

use crate::viewport_presentation::PresentationMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProfileDiagnosticStatus {
    Ready,
    Missing,
    Unsupported,
    AmbiguousDisplay,
    HdrSdrMismatch,
    ExplicitFallback,
    StaleGeneration,
}

impl ProfileDiagnosticStatus {
    #[must_use]
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Ready => "Display profile ready",
            Self::Missing => "Display profile missing",
            Self::Unsupported => "Display profile unsupported",
            Self::AmbiguousDisplay => "Display selection ambiguous",
            Self::HdrSdrMismatch => "HDR/SDR mismatch",
            Self::ExplicitFallback => "Explicit display fallback",
            Self::StaleGeneration => "Display profile stale",
        }
    }

    #[must_use]
    const fn css_class(self) -> &'static str {
        match self {
            Self::Ready => "profile-diagnostic-ready",
            Self::Missing => "profile-diagnostic-missing",
            Self::Unsupported => "profile-diagnostic-unsupported",
            Self::AmbiguousDisplay => "profile-diagnostic-ambiguous",
            Self::HdrSdrMismatch => "profile-diagnostic-range-mismatch",
            Self::ExplicitFallback => "profile-diagnostic-fallback",
            Self::StaleGeneration => "profile-diagnostic-stale",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProfileDiagnosticRequest {
    expected_generation: Option<u64>,
    presentation_mode: Option<PresentationMode>,
}

impl ProfileDiagnosticRequest {
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self {
            expected_generation: None,
            presentation_mode: None,
        }
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn for_generation(expected_generation: u64) -> Self {
        Self {
            expected_generation: Some(expected_generation),
            presentation_mode: None,
        }
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn with_presentation_mode(mut self, mode: PresentationMode) -> Self {
        self.presentation_mode = Some(mode);
        self
    }
}

impl Default for ProfileDiagnosticRequest {
    fn default() -> Self {
        Self::new()
    }
}

/// The immutable facts needed by the UI projection. All values originate from typed service
/// decisions; no profile identity or profile payload is retained here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProfileDiagnosticFacts {
    selection: ProfileSelection,
    status: SelectionStatus,
    hdr: HdrCapability,
    generation: u64,
}

impl ProfileDiagnosticFacts {
    #[must_use]
    pub(crate) const fn new(
        selection: ProfileSelection,
        status: SelectionStatus,
        hdr: HdrCapability,
        generation: u64,
    ) -> Self {
        Self {
            selection,
            status,
            hdr,
            generation,
        }
    }
}

impl From<&DisplayProfileSnapshot> for ProfileDiagnosticFacts {
    fn from(snapshot: &DisplayProfileSnapshot) -> Self {
        Self::new(
            snapshot.selection(),
            snapshot.status(),
            snapshot.hdr_capability(),
            snapshot.generation(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProfileDiagnosticProjection {
    status: ProfileDiagnosticStatus,
    detail: String,
    generation: Option<u64>,
}

impl ProfileDiagnosticProjection {
    #[must_use]
    pub(crate) fn detail(&self) -> &str {
        &self.detail
    }

    #[must_use]
    pub(crate) fn text(&self) -> String {
        format!("{} · {}", self.status.label(), self.detail)
    }
}

/// Projects a snapshot/receipt pair into a visible status without doing profile work.
#[must_use]
pub(crate) fn project_profile_diagnostic(
    snapshot: Option<&DisplayProfileSnapshot>,
    receipt: Option<DisplayProfileReceipt>,
    request: ProfileDiagnosticRequest,
) -> ProfileDiagnosticProjection {
    let facts = snapshot.map(ProfileDiagnosticFacts::from);
    project_facts(facts, receipt, request)
}

/// Projects already-extracted typed profile facts. This keeps the decision table display-free and
/// makes it straightforward for non-GTK callers to verify the same status contract.
#[must_use]
pub(crate) fn project_facts(
    facts: Option<ProfileDiagnosticFacts>,
    receipt: Option<DisplayProfileReceipt>,
    request: ProfileDiagnosticRequest,
) -> ProfileDiagnosticProjection {
    let Some(facts) = facts else {
        return match receipt {
            Some(receipt) if receipt.monitor_count() > 1 => projection(
                ProfileDiagnosticStatus::AmbiguousDisplay,
                format!(
                    "{} active displays have no unambiguous target",
                    receipt.monitor_count()
                ),
                Some(receipt.generation()),
            ),
            Some(receipt) => projection(
                ProfileDiagnosticStatus::Missing,
                if receipt.monitor_count() == 0 {
                    "no active display profile evidence".to_owned()
                } else {
                    "no usable profile for the active display".to_owned()
                },
                Some(receipt.generation()),
            ),
            None => projection(
                ProfileDiagnosticStatus::Missing,
                "no active display profile evidence".to_owned(),
                None,
            ),
        };
    };

    if let Some(expected) = request.expected_generation
        && expected != facts.generation
    {
        return stale_projection(facts.generation, expected);
    }
    if let Some(receipt) = receipt
        && receipt.generation() != facts.generation
    {
        return stale_projection(facts.generation, receipt.generation());
    }

    let generation = Some(facts.generation);
    match facts.status {
        SelectionStatus::Stale(reason) => projection(
            ProfileDiagnosticStatus::Unsupported,
            format!(
                "{} · {}",
                stale_reason_label(reason),
                generation_label(facts.generation)
            ),
            generation,
        ),
        SelectionStatus::Degraded(reason) => degraded_projection(reason, generation),
        SelectionStatus::Active => {
            if facts.selection == ProfileSelection::Unprofiled {
                return projection(
                    ProfileDiagnosticStatus::Missing,
                    format!("unprofiled · {}", generation_label(facts.generation)),
                    generation,
                );
            }
            if let Some(mode) = request.presentation_mode
                && mode_mismatches(mode, facts.hdr)
            {
                return projection(
                    ProfileDiagnosticStatus::HdrSdrMismatch,
                    format!(
                        "{} presentation on {} display · {}",
                        mode.label(),
                        display_range_label(facts.hdr),
                        generation_label(facts.generation)
                    ),
                    generation,
                );
            }
            if facts.selection == ProfileSelection::UserFallback {
                return projection(
                    ProfileDiagnosticStatus::ExplicitFallback,
                    format!(
                        "user fallback profile · {}",
                        generation_label(facts.generation)
                    ),
                    generation,
                );
            }
            projection(
                ProfileDiagnosticStatus::Ready,
                format!(
                    "{} · {}",
                    selection_label(facts.selection),
                    generation_label(facts.generation)
                ),
                generation,
            )
        }
    }
}

fn projection(
    status: ProfileDiagnosticStatus,
    detail: String,
    generation: Option<u64>,
) -> ProfileDiagnosticProjection {
    ProfileDiagnosticProjection {
        status,
        detail,
        generation,
    }
}

fn stale_projection(
    snapshot_generation: u64,
    current_generation: u64,
) -> ProfileDiagnosticProjection {
    projection(
        ProfileDiagnosticStatus::StaleGeneration,
        format!(
            "snapshot generation {snapshot_generation}; current generation {current_generation}"
        ),
        Some(snapshot_generation),
    )
}

fn degraded_projection(
    reason: DegradedReason,
    generation: Option<u64>,
) -> ProfileDiagnosticProjection {
    let (status, detail) = match reason {
        DegradedReason::ProviderUnavailable => (
            ProfileDiagnosticStatus::Missing,
            "display profile provider unavailable".to_owned(),
        ),
        DegradedReason::ProfileAbsent => (
            ProfileDiagnosticStatus::Missing,
            "display has no profile".to_owned(),
        ),
        DegradedReason::MonitorUnresolved | DegradedReason::MonitorRemoved => (
            ProfileDiagnosticStatus::AmbiguousDisplay,
            "active display could not be resolved unambiguously".to_owned(),
        ),
        DegradedReason::PermissionDenied | DegradedReason::ProfileUnreadable => (
            ProfileDiagnosticStatus::Unsupported,
            "display profile could not be read".to_owned(),
        ),
    };
    projection(status, detail, generation)
}

fn mode_mismatches(mode: PresentationMode, hdr: HdrCapability) -> bool {
    matches!(
        (mode, hdr.active),
        (PresentationMode::Hdr, false) | (PresentationMode::Sdr, true)
    )
}

const fn display_range_label(hdr: HdrCapability) -> &'static str {
    if hdr.active { "HDR" } else { "SDR" }
}

const fn selection_label(selection: ProfileSelection) -> &'static str {
    match selection {
        ProfileSelection::Override => "explicit override",
        ProfileSelection::OperatingSystem => "operating-system profile",
        ProfileSelection::UserFallback => "explicit fallback",
        ProfileSelection::Unprofiled => "unprofiled",
    }
}

const fn stale_reason_label(reason: StaleReason) -> &'static str {
    match reason {
        StaleReason::ProfileReadFailed => "profile read failed",
        StaleReason::ProfileInvalid => "profile invalid",
        StaleReason::ProfileUnsupported => "profile format unsupported",
        StaleReason::ProfileOversized => "profile exceeds the supported size",
        StaleReason::ProfileChangedDuringRead => "profile changed during acquisition",
    }
}

fn generation_label(generation: u64) -> String {
    format!("generation {generation}")
}

#[derive(Clone)]
pub(crate) struct ProfileDiagnosticSurface {
    root: gtk4::Box,
    label: gtk4::Label,
}

impl ProfileDiagnosticSurface {
    #[must_use]
    pub(crate) fn new(widget_name: &str, accessible_name: &str) -> Self {
        let root = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        root.set_widget_name(widget_name);
        root.add_css_class("dt_profile_diagnostic");
        root.set_hexpand(true);
        root.set_halign(gtk4::Align::Fill);

        let label = gtk4::Label::new(Some("Display profile missing"));
        label.set_halign(gtk4::Align::Start);
        label.set_hexpand(true);
        label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        label.set_accessible_role(gtk4::AccessibleRole::Status);
        label.update_property(&[Property::Label(accessible_name)]);
        root.append(&label);

        let surface = Self { root, label };
        surface.set_projection(&project_profile_diagnostic(
            None,
            None,
            ProfileDiagnosticRequest::new(),
        ));
        surface
    }

    #[must_use]
    pub(crate) const fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    pub(crate) fn set_projection(&self, projection: &ProfileDiagnosticProjection) {
        for status in [
            ProfileDiagnosticStatus::Ready,
            ProfileDiagnosticStatus::Missing,
            ProfileDiagnosticStatus::Unsupported,
            ProfileDiagnosticStatus::AmbiguousDisplay,
            ProfileDiagnosticStatus::HdrSdrMismatch,
            ProfileDiagnosticStatus::ExplicitFallback,
            ProfileDiagnosticStatus::StaleGeneration,
        ] {
            self.root.remove_css_class(status.css_class());
        }
        self.root.add_css_class(projection.status.css_class());
        self.label.set_text(&projection.text());
        self.label.set_tooltip_text(Some(projection.detail()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusttable_display_profile::{
        DisplayProfileService, DisplayProvider, MonitorDescriptor, MonitorGeometry, MonitorId,
        ProfileProbe, ProviderMonitor,
    };

    fn facts(selection: ProfileSelection, status: SelectionStatus) -> ProfileDiagnosticFacts {
        ProfileDiagnosticFacts::new(
            selection,
            status,
            HdrCapability {
                supported: true,
                active: false,
            },
            7,
        )
    }

    #[test]
    fn projects_the_typed_status_matrix() {
        let cases = [
            (
                facts(ProfileSelection::OperatingSystem, SelectionStatus::Active),
                ProfileDiagnosticStatus::Ready,
            ),
            (
                facts(
                    ProfileSelection::Unprofiled,
                    SelectionStatus::Degraded(DegradedReason::ProfileAbsent),
                ),
                ProfileDiagnosticStatus::Missing,
            ),
            (
                facts(
                    ProfileSelection::OperatingSystem,
                    SelectionStatus::Stale(StaleReason::ProfileUnsupported),
                ),
                ProfileDiagnosticStatus::Unsupported,
            ),
            (
                facts(
                    ProfileSelection::Unprofiled,
                    SelectionStatus::Degraded(DegradedReason::MonitorUnresolved),
                ),
                ProfileDiagnosticStatus::AmbiguousDisplay,
            ),
            (
                facts(ProfileSelection::UserFallback, SelectionStatus::Active),
                ProfileDiagnosticStatus::ExplicitFallback,
            ),
        ];

        for (facts, expected) in cases {
            assert_eq!(
                project_facts(Some(facts), None, ProfileDiagnosticRequest::new()).status,
                expected
            );
        }
    }

    #[test]
    fn projects_hdr_sdr_mismatch_without_touching_profile_data() {
        let facts = ProfileDiagnosticFacts::new(
            ProfileSelection::OperatingSystem,
            SelectionStatus::Active,
            HdrCapability {
                supported: true,
                active: false,
            },
            7,
        );
        let request = ProfileDiagnosticRequest::new().with_presentation_mode(PresentationMode::Hdr);

        assert_eq!(
            project_facts(Some(facts), None, request).status,
            ProfileDiagnosticStatus::HdrSdrMismatch
        );
    }

    #[test]
    fn projects_ambiguous_and_stale_receipt_states() {
        let missing_receipt = receipt_for_monitors(0);
        assert_eq!(
            project_facts(None, Some(missing_receipt), ProfileDiagnosticRequest::new()).status,
            ProfileDiagnosticStatus::Missing
        );

        let ambiguous_receipt = receipt_for_monitors(2);
        assert_eq!(
            project_facts(
                None,
                Some(ambiguous_receipt),
                ProfileDiagnosticRequest::new()
            )
            .status,
            ProfileDiagnosticStatus::AmbiguousDisplay
        );

        let facts = facts(ProfileSelection::OperatingSystem, SelectionStatus::Active);
        assert_eq!(
            project_facts(
                Some(facts),
                None,
                ProfileDiagnosticRequest::for_generation(8)
            )
            .status,
            ProfileDiagnosticStatus::StaleGeneration
        );
    }

    fn receipt_for_monitors(count: usize) -> DisplayProfileReceipt {
        let monitors = (0..count)
            .map(|index| {
                let id = MonitorId::from_platform_parts(
                    "test",
                    Some(&index.to_string()),
                    None,
                    None,
                    None,
                );
                let descriptor = MonitorDescriptor::new(
                    id,
                    format!("Display {index}"),
                    MonitorGeometry::new(0, 0, 1, 1, 1).expect("valid test geometry"),
                    rusttable_display_profile::HdrDescriptor {
                        supported: false,
                        active: false,
                    },
                )
                .expect("valid test descriptor");
                ProviderMonitor::new(descriptor, DisplayProvider::Synthetic, ProfileProbe::Absent)
            })
            .collect::<Vec<_>>();
        DisplayProfileService::new()
            .reconcile(monitors)
            .expect("test inventory is valid")
    }
}
