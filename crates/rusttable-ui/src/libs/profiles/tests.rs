use super::*;

fn unresolved_roles() -> [ProfileRoleState; PROFILE_ROLE_COUNT] {
    ProfileRole::all()
        .map(|role| ProfileRoleState::unavailable(role, ProfileUnavailableReason::NotResolved))
}

#[test]
fn roles_map_to_workspace_color_roles() {
    assert_eq!(ProfileRole::Input.color_role(), ColorRole::Input);
    assert_eq!(ProfileRole::SoftProof.color_role(), ColorRole::Proof);
    assert_eq!(ProfileRole::Export.color_role(), ColorRole::Export);
    assert_eq!(ProfileRole::all().len(), PROFILE_ROLE_COUNT);
}

#[test]
fn ready_unavailable_and_mismatch_states_remain_truthful() {
    let ready = ProfileRoleState::ready(ProfileRole::Working, ColorEncoding::AcesCgD60);
    assert_eq!(ready.profile(), Some(ColorEncoding::AcesCgD60));
    assert!(ready.status().is_ready());
    let unspecified = ProfileRoleState::ready(ProfileRole::Input, ColorEncoding::Unspecified);
    assert_eq!(unspecified.profile(), None);
    assert!(!unspecified.status().is_ready());
    for state in [
        ProfileRoleState::unavailable(ProfileRole::Display, ProfileUnavailableReason::Missing),
        ProfileRoleState::mismatch(ProfileRole::Display, ProfileMismatchKind::HdrSdr),
    ] {
        assert_eq!(state.profile(), None);
        assert!(!state.status().is_ready());
    }
}

#[test]
fn service_statuses_project_without_parsing_or_fallback_invention() {
    assert_eq!(
        display_status(
            SelectionStatus::Stale(StaleReason::ProfileUnsupported),
            ProviderAvailability::Available,
            false,
        ),
        ProfileRoleStatus::Unavailable(ProfileUnavailableReason::Unsupported)
    );
    assert_eq!(
        display_status(
            SelectionStatus::Degraded(DegradedReason::MonitorUnresolved),
            ProviderAvailability::Available,
            false,
        ),
        ProfileRoleStatus::Unavailable(ProfileUnavailableReason::NoActiveMonitor)
    );
    let state = ProfileControlsState::from_display_snapshot(7, None);
    assert_eq!(state.role(ProfileRole::Display).profile(), None);
    assert!(!state.role(ProfileRole::Display).status().is_ready());
}

#[test]
fn warnings_are_bounded_stable_and_include_explicit_fallback() {
    let roles = ProfileRole::all().map(|role| {
        let state = ProfileRoleState::mismatch(role, ProfileMismatchKind::Encoding);
        if role == ProfileRole::Input {
            state.with_source(ProfileSelection::UserFallback)
        } else {
            state
        }
    });
    let state = ProfileControlsState::new(
        4,
        roles,
        RenderingIntent::Perceptual,
        BlackPointCompensation::Enabled,
        true,
        true,
    );
    assert_eq!(state.warnings().len(), MAX_PROFILE_WARNINGS);
    assert_eq!(state.warnings()[0].kind(), ProfileWarningKind::Mismatch);
    assert_eq!(
        state.warnings()[1].kind(),
        ProfileWarningKind::ExplicitFallback
    );
    assert!(ProfileWarningKind::HdrSdrMismatch.blocking());
    assert!(!ProfileWarningKind::ExplicitFallback.blocking());
}

#[test]
fn old_messages_and_snapshots_are_rejected_by_generation() {
    let mut state = ProfileControlsState::new(
        9,
        unresolved_roles(),
        RenderingIntent::Relative,
        BlackPointCompensation::Disabled,
        false,
        false,
    );
    let old = ProfileControlMessage::new(8, ProfileControlAction::SetSoftProof(true));
    let current = ProfileControlMessage::new(9, ProfileControlAction::SetSoftProof(true));
    assert!(!state.accepts_message(&old));
    assert!(state.accepts_message(&current));
    assert!(!state.apply_display_snapshot(8, None));
    assert!(state.apply_display_snapshot(10, None));
    assert_eq!(state.generation(), 10);
}

#[test]
fn warning_text_and_accessibility_contracts_are_bounded() {
    for kind in [
        ProfileWarningKind::Missing,
        ProfileWarningKind::Unsupported,
        ProfileWarningKind::AmbiguousDisplay,
        ProfileWarningKind::HdrSdrMismatch,
    ] {
        assert!(kind.message().len() < 120);
        assert!(!kind.message().contains("/Users/"));
        assert!(!kind.message().contains('\\'));
    }
    assert_eq!(
        PROFILE_CONTROLS_FOCUS_ORDER,
        PROFILE_CONTROL_WIDGET_IDS[1..10]
    );
    assert!(
        ProfileRole::all()
            .iter()
            .all(|role| !role.label().is_empty())
    );
}
