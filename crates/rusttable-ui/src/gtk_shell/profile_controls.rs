//! Typed GTK4 controls for color-profile selection and proofing.
//!
//! This is a presentation boundary: callers provide resolved profile identities and service
//! status, and receive typed intent. No profile bytes, parsing, resolution, or transforms live
//! here.

use gtk4::accessible::Property;
use gtk4::prelude::*;
use rusttable_color::{BlackPointCompensation, ColorEncoding, ColorRole, RenderingIntent};
use rusttable_display_profile::{
    DegradedReason, DisplayProfileSnapshot, ProfileSelection, ProviderAvailability,
    SelectionStatus, StaleReason,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const MAX_PROFILE_WARNINGS: usize = 3;
pub const PROFILE_CONTROL_WIDGET_IDS: [&str; 15] = [
    "color-profile-controls",
    "color-profile-input",
    "color-profile-working",
    "color-profile-display",
    "color-profile-soft-proof",
    "color-profile-export",
    "color-profile-intent",
    "color-profile-bpc",
    "color-profile-soft-proof-toggle",
    "color-profile-gamut-warning",
    "color-profile-warning-0",
    "color-profile-warning-1",
    "color-profile-warning-2",
    "color-profile-status",
    "color-profile-generation",
];
pub const PROFILE_CONTROLS_FOCUS_ORDER: [&str; 9] = [
    "color-profile-input",
    "color-profile-working",
    "color-profile-display",
    "color-profile-soft-proof",
    "color-profile-export",
    "color-profile-intent",
    "color-profile-bpc",
    "color-profile-soft-proof-toggle",
    "color-profile-gamut-warning",
];
const PROFILE_ROLE_COUNT: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProfileRole {
    Input,
    Working,
    Display,
    SoftProof,
    Export,
}

impl ProfileRole {
    const ALL: [Self; PROFILE_ROLE_COUNT] = [
        Self::Input,
        Self::Working,
        Self::Display,
        Self::SoftProof,
        Self::Export,
    ];
    #[must_use]
    pub const fn all() -> [Self; PROFILE_ROLE_COUNT] {
        Self::ALL
    }
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Input => "input profile",
            Self::Working => "working profile",
            Self::Display => "display profile",
            Self::SoftProof => "soft-proof profile",
            Self::Export => "export profile",
        }
    }
    #[must_use]
    pub const fn color_role(self) -> ColorRole {
        match self {
            Self::Input => ColorRole::Input,
            Self::Working => ColorRole::Working,
            Self::Display => ColorRole::Display,
            Self::SoftProof => ColorRole::Proof,
            Self::Export => ColorRole::Export,
        }
    }

    const fn index(self) -> usize {
        match self {
            Self::Input => 0,
            Self::Working => 1,
            Self::Display => 2,
            Self::SoftProof => 3,
            Self::Export => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileUnavailableReason {
    NotResolved,
    NoActiveMonitor,
    Missing,
    Unsupported,
    ProviderUnavailable,
    Stale,
}

impl ProfileUnavailableReason {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::NotResolved => "not resolved",
            Self::NoActiveMonitor => "no active monitor",
            Self::Missing => "unavailable",
            Self::Unsupported => "unsupported",
            Self::ProviderUnavailable => "provider unavailable",
            Self::Stale => "stale profile",
        }
    }
    const fn warning_kind(self) -> Option<ProfileWarningKind> {
        match self {
            Self::NotResolved => None,
            Self::NoActiveMonitor => Some(ProfileWarningKind::AmbiguousDisplay),
            Self::Missing => Some(ProfileWarningKind::Missing),
            Self::Unsupported => Some(ProfileWarningKind::Unsupported),
            Self::ProviderUnavailable => Some(ProfileWarningKind::ProviderUnavailable),
            Self::Stale => Some(ProfileWarningKind::Stale),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileMismatchKind {
    Encoding,
    HdrSdr,
    Selection,
}

impl ProfileMismatchKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Encoding => "encoding mismatch",
            Self::HdrSdr => "HDR/SDR mismatch",
            Self::Selection => "selection mismatch",
        }
    }

    const fn warning_kind(self) -> ProfileWarningKind {
        match self {
            Self::Encoding | Self::Selection => ProfileWarningKind::Mismatch,
            Self::HdrSdr => ProfileWarningKind::HdrSdrMismatch,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileRoleStatus {
    Ready,
    Unavailable(ProfileUnavailableReason),
    Mismatch(ProfileMismatchKind),
}

impl ProfileRoleStatus {
    #[must_use]
    pub const fn is_ready(self) -> bool {
        matches!(self, Self::Ready)
    }
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Unavailable(reason) => reason.label(),
            Self::Mismatch(kind) => kind.label(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileChoice {
    profile: ColorEncoding,
    label: String,
}

impl ProfileChoice {
    #[must_use]
    pub fn new(profile: ColorEncoding, label: impl Into<String>) -> Self {
        Self {
            profile,
            label: label.into(),
        }
    }
    #[must_use]
    pub const fn profile(&self) -> ColorEncoding {
        self.profile
    }
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileRoleState {
    role: ProfileRole,
    profile: Option<ColorEncoding>,
    status: ProfileRoleStatus,
    source: Option<ProfileSelection>,
    choices: Vec<ProfileChoice>,
}

impl ProfileRoleState {
    #[must_use]
    pub fn ready(role: ProfileRole, profile: ColorEncoding) -> Self {
        if profile == ColorEncoding::Unspecified {
            return Self::unavailable(role, ProfileUnavailableReason::NotResolved);
        }
        Self {
            role,
            profile: Some(profile),
            status: ProfileRoleStatus::Ready,
            source: None,
            choices: vec![ProfileChoice::new(profile, profile_label(profile))],
        }
    }

    #[must_use]
    pub const fn unavailable(role: ProfileRole, reason: ProfileUnavailableReason) -> Self {
        Self {
            role,
            profile: None,
            status: ProfileRoleStatus::Unavailable(reason),
            source: None,
            choices: Vec::new(),
        }
    }

    #[must_use]
    pub const fn mismatch(role: ProfileRole, kind: ProfileMismatchKind) -> Self {
        Self {
            role,
            profile: None,
            status: ProfileRoleStatus::Mismatch(kind),
            source: None,
            choices: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_choices(mut self, choices: impl IntoIterator<Item = ProfileChoice>) -> Self {
        self.choices = choices.into_iter().collect();
        self
    }
    #[must_use]
    pub const fn with_source(mut self, source: ProfileSelection) -> Self {
        self.source = Some(source);
        self
    }
    #[must_use]
    pub const fn role(&self) -> ProfileRole {
        self.role
    }
    #[must_use]
    pub const fn profile(&self) -> Option<ColorEncoding> {
        self.profile
    }
    #[must_use]
    pub const fn status(&self) -> ProfileRoleStatus {
        self.status
    }
    #[must_use]
    pub const fn source(&self) -> Option<ProfileSelection> {
        self.source
    }
    #[must_use]
    pub fn choices(&self) -> &[ProfileChoice] {
        &self.choices
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileWarningKind {
    Missing,
    Unsupported,
    ProviderUnavailable,
    AmbiguousDisplay,
    Stale,
    Mismatch,
    HdrSdrMismatch,
    ExplicitFallback,
}

impl ProfileWarningKind {
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Missing => "Profile is unavailable; rendering and export remain unresolved.",
            Self::Unsupported => "Profile is unsupported; choose a supported profile.",
            Self::ProviderUnavailable => "Display profile service is unavailable.",
            Self::AmbiguousDisplay => "Display profile is unresolved for the active monitor.",
            Self::Stale => "Display profile changed or became invalid; refresh the decision.",
            Self::Mismatch => "Profile selection does not match the requested color role.",
            Self::HdrSdrMismatch => "HDR/SDR display mode and profile do not match.",
            Self::ExplicitFallback => "An explicit fallback profile is active.",
        }
    }

    #[must_use]
    pub const fn blocking(self) -> bool {
        !matches!(self, Self::ExplicitFallback)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileWarning {
    role: ProfileRole,
    kind: ProfileWarningKind,
}

impl ProfileWarning {
    #[must_use]
    pub const fn new(role: ProfileRole, kind: ProfileWarningKind) -> Self {
        Self { role, kind }
    }

    #[must_use]
    pub const fn role(self) -> ProfileRole {
        self.role
    }
    #[must_use]
    pub const fn kind(self) -> ProfileWarningKind {
        self.kind
    }
    #[must_use]
    pub const fn message(self) -> &'static str {
        self.kind.message()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileControlsState {
    generation: u64,
    roles: [ProfileRoleState; PROFILE_ROLE_COUNT],
    intent: RenderingIntent,
    black_point_compensation: BlackPointCompensation,
    soft_proof_enabled: bool,
    gamut_warning_enabled: bool,
    warnings: Vec<ProfileWarning>,
}

impl ProfileControlsState {
    #[must_use]
    pub fn new(
        generation: u64,
        roles: [ProfileRoleState; PROFILE_ROLE_COUNT],
        intent: RenderingIntent,
        black_point_compensation: BlackPointCompensation,
        soft_proof_enabled: bool,
        gamut_warning_enabled: bool,
    ) -> Self {
        let mut state = Self {
            generation,
            roles,
            intent,
            black_point_compensation,
            soft_proof_enabled,
            gamut_warning_enabled,
            warnings: Vec::with_capacity(MAX_PROFILE_WARNINGS),
        };
        state.rebuild_warnings();
        state
    }

    #[must_use]
    pub fn from_display_snapshot(
        generation: u64,
        snapshot: Option<&DisplayProfileSnapshot>,
    ) -> Self {
        let mut roles = ProfileRole::ALL
            .map(|role| ProfileRoleState::unavailable(role, ProfileUnavailableReason::NotResolved));
        roles[ProfileRole::Display.index()] = snapshot.map_or_else(
            || {
                ProfileRoleState::unavailable(
                    ProfileRole::Display,
                    ProfileUnavailableReason::NoActiveMonitor,
                )
            },
            display_role_state,
        );
        Self::new(
            generation,
            roles,
            RenderingIntent::Relative,
            BlackPointCompensation::Disabled,
            false,
            false,
        )
    }

    #[must_use]
    pub fn apply_display_snapshot(
        &mut self,
        generation: u64,
        snapshot: Option<&DisplayProfileSnapshot>,
    ) -> bool {
        if generation < self.generation {
            return false;
        }
        self.generation = generation;
        self.roles[ProfileRole::Display.index()] = snapshot.map_or_else(
            || {
                ProfileRoleState::unavailable(
                    ProfileRole::Display,
                    ProfileUnavailableReason::NoActiveMonitor,
                )
            },
            display_role_state,
        );
        self.rebuild_warnings();
        true
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    #[must_use]
    pub fn role(&self, role: ProfileRole) -> &ProfileRoleState {
        &self.roles[role.index()]
    }
    #[must_use]
    pub const fn intent(&self) -> RenderingIntent {
        self.intent
    }
    #[must_use]
    pub const fn black_point_compensation(&self) -> BlackPointCompensation {
        self.black_point_compensation
    }
    #[must_use]
    pub const fn soft_proof_enabled(&self) -> bool {
        self.soft_proof_enabled
    }
    #[must_use]
    pub const fn gamut_warning_enabled(&self) -> bool {
        self.gamut_warning_enabled
    }
    #[must_use]
    pub fn warnings(&self) -> &[ProfileWarning] {
        &self.warnings
    }
    #[must_use]
    pub fn accepts_message(&self, message: &ProfileControlMessage) -> bool {
        message.generation() == self.generation
    }

    fn rebuild_warnings(&mut self) {
        self.warnings.clear();
        for role in ProfileRole::ALL {
            let (status, source) = {
                let state = self.role(role);
                (state.status(), state.source())
            };
            if let Some(kind) = status_warning(status) {
                self.push_warning(ProfileWarning::new(role, kind));
            }
            if source == Some(ProfileSelection::UserFallback) {
                self.push_warning(ProfileWarning::new(
                    role,
                    ProfileWarningKind::ExplicitFallback,
                ));
            }
        }
    }

    fn push_warning(&mut self, warning: ProfileWarning) {
        if self.warnings.len() < MAX_PROFILE_WARNINGS {
            self.warnings.push(warning);
        }
    }
}

impl Default for ProfileControlsState {
    fn default() -> Self {
        Self::new(
            0,
            ProfileRole::ALL.map(|role| {
                ProfileRoleState::unavailable(role, ProfileUnavailableReason::NotResolved)
            }),
            RenderingIntent::Relative,
            BlackPointCompensation::Disabled,
            false,
            false,
        )
    }
}
fn status_warning(status: ProfileRoleStatus) -> Option<ProfileWarningKind> {
    match status {
        ProfileRoleStatus::Ready => None,
        ProfileRoleStatus::Unavailable(reason) => reason.warning_kind(),
        ProfileRoleStatus::Mismatch(kind) => Some(kind.warning_kind()),
    }
}
fn display_status(
    status: SelectionStatus,
    availability: ProviderAvailability,
    has_profile: bool,
) -> ProfileRoleStatus {
    if availability == ProviderAvailability::Unavailable {
        return ProfileRoleStatus::Unavailable(ProfileUnavailableReason::ProviderUnavailable);
    }
    match status {
        SelectionStatus::Active if has_profile => ProfileRoleStatus::Ready,
        SelectionStatus::Active | SelectionStatus::Degraded(DegradedReason::ProfileAbsent) => {
            ProfileRoleStatus::Unavailable(ProfileUnavailableReason::Missing)
        }
        SelectionStatus::Stale(StaleReason::ProfileUnsupported) => {
            ProfileRoleStatus::Unavailable(ProfileUnavailableReason::Unsupported)
        }
        SelectionStatus::Stale(_) => {
            ProfileRoleStatus::Unavailable(ProfileUnavailableReason::Stale)
        }
        SelectionStatus::Degraded(DegradedReason::ProviderUnavailable) => {
            ProfileRoleStatus::Unavailable(ProfileUnavailableReason::ProviderUnavailable)
        }
        SelectionStatus::Degraded(DegradedReason::MonitorUnresolved) => {
            ProfileRoleStatus::Unavailable(ProfileUnavailableReason::NoActiveMonitor)
        }
        SelectionStatus::Degraded(
            DegradedReason::PermissionDenied
            | DegradedReason::ProfileUnreadable
            | DegradedReason::MonitorRemoved,
        ) => ProfileRoleStatus::Unavailable(ProfileUnavailableReason::Stale),
    }
}
fn display_role_state(snapshot: &DisplayProfileSnapshot) -> ProfileRoleState {
    let status = display_status(
        snapshot.status(),
        snapshot.provider_availability(),
        snapshot.profile_id().is_some(),
    );
    let mut state = match (status, snapshot.profile_id()) {
        (ProfileRoleStatus::Ready, Some(profile)) => {
            ProfileRoleState::ready(ProfileRole::Display, ColorEncoding::External(profile))
        }
        (ProfileRoleStatus::Unavailable(reason), _) => {
            ProfileRoleState::unavailable(ProfileRole::Display, reason)
        }
        (ProfileRoleStatus::Mismatch(kind), _) => {
            ProfileRoleState::mismatch(ProfileRole::Display, kind)
        }
        (ProfileRoleStatus::Ready, None) => {
            ProfileRoleState::unavailable(ProfileRole::Display, ProfileUnavailableReason::Missing)
        }
    };
    if status.is_ready() {
        state = state.with_source(snapshot.selection());
    }
    state
}
fn profile_label(profile: ColorEncoding) -> &'static str {
    match profile {
        ColorEncoding::Unspecified => "unspecified",
        ColorEncoding::SrgbD65 => "sRGB",
        ColorEncoding::DisplayP3D65 => "Display P3",
        ColorEncoding::LinearSrgbD65 => "linear sRGB",
        ColorEncoding::LinearDisplayP3D65 => "linear Display P3",
        ColorEncoding::Rec2020D65 => "Rec. 2020",
        ColorEncoding::LinearRec2020D65 => "linear Rec. 2020",
        ColorEncoding::AcesCgD60 => "ACEScg",
        ColorEncoding::XyzD50 => "XYZ D50",
        ColorEncoding::XyzD65 => "XYZ D65",
        ColorEncoding::LabD50 => "Lab D50",
        ColorEncoding::LchD50 => "LCh D50",
        ColorEncoding::External(_) => "external profile",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileControlAction {
    SelectProfile {
        role: ProfileRole,
        profile: ColorEncoding,
    },
    SetIntent(RenderingIntent),
    SetBlackPointCompensation(BlackPointCompensation),
    SetSoftProof(bool),
    SetGamutWarning(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileControlMessage {
    generation: u64,
    action: ProfileControlAction,
}

impl ProfileControlMessage {
    #[must_use]
    pub const fn new(generation: u64, action: ProfileControlAction) -> Self {
        Self { generation, action }
    }
    #[must_use]
    pub const fn generation(self) -> u64 {
        self.generation
    }
    #[must_use]
    pub const fn action(self) -> ProfileControlAction {
        self.action
    }
}

type ProfileMessageHandler = Box<dyn Fn(ProfileControlMessage)>;

#[derive(Clone)]
struct ProfileRow {
    role: ProfileRole,
    selector: gtk4::DropDown,
    status: gtk4::Label,
}

#[derive(Clone)]
pub struct ProfileControls {
    root: gtk4::Expander,
    rows: Vec<ProfileRow>,
    intent: gtk4::DropDown,
    bpc: gtk4::CheckButton,
    soft_proof: gtk4::ToggleButton,
    gamut_warning: gtk4::ToggleButton,
    status: gtk4::Label,
    warning_rows: Vec<gtk4::Label>,
    generation: gtk4::Label,
    state: Rc<RefCell<ProfileControlsState>>,
    sync_guard: Rc<Cell<bool>>,
    handler: Rc<RefCell<Option<ProfileMessageHandler>>>,
}

impl ProfileControls {
    #[must_use]
    pub fn new(initial: ProfileControlsState) -> Self {
        let state = Rc::new(RefCell::new(initial));
        let sync_guard = Rc::new(Cell::new(false));
        let handler = Rc::new(RefCell::new(None));
        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 3);
        content.set_widget_name("color-profile-controls-content");
        let rows = ProfileRole::ALL
            .map(|role| profile_row(role, &content))
            .into_iter()
            .collect();

        let intent = gtk4::DropDown::from_strings(&[
            "perceptual",
            "relative colorimetric",
            "saturation",
            "absolute colorimetric",
        ]);
        setup_accessible(&intent, PROFILE_CONTROL_WIDGET_IDS[6], "Rendering intent");
        append_labeled(&content, "rendering intent", &intent);

        let bpc = gtk4::CheckButton::with_label("black-point compensation");
        setup_accessible(
            &bpc,
            PROFILE_CONTROL_WIDGET_IDS[7],
            "Black-point compensation",
        );
        bpc.set_accessible_role(gtk4::AccessibleRole::Checkbox);
        content.append(&bpc);

        let soft_proof = gtk4::ToggleButton::with_label("soft proof");
        setup_toggle(
            &soft_proof,
            PROFILE_CONTROL_WIDGET_IDS[8],
            "Toggle soft proof using the resolved soft-proof profile",
        );
        content.append(&soft_proof);
        let gamut_warning = gtk4::ToggleButton::with_label("gamut warning");
        setup_toggle(
            &gamut_warning,
            PROFILE_CONTROL_WIDGET_IDS[9],
            "Toggle gamut warning using the resolved display and proof profiles",
        );
        content.append(&gamut_warning);

        let status = status_label(PROFILE_CONTROL_WIDGET_IDS[13], "Color profile status");
        content.append(&status);
        let warning_rows = (0..MAX_PROFILE_WARNINGS)
            .map(|index| {
                let row = status_label(
                    PROFILE_CONTROL_WIDGET_IDS[10 + index],
                    "Color profile warning",
                );
                row.add_css_class("warning");
                content.append(&row);
                row
            })
            .collect();
        let generation = status_label(
            PROFILE_CONTROL_WIDGET_IDS[14],
            "Profile decision generation",
        );
        generation.add_css_class("dim-label");
        content.append(&generation);

        let root = gtk4::Expander::builder()
            .label("color profiles")
            .expanded(true)
            .child(&content)
            .build();
        setup_accessible(
            &root,
            PROFILE_CONTROL_WIDGET_IDS[0],
            "Color profiles and soft proofing",
        );
        root.set_accessible_role(gtk4::AccessibleRole::Group);
        root.set_hexpand(true);

        let controls = Self {
            root,
            rows,
            intent,
            bpc,
            soft_proof,
            gamut_warning,
            status,
            warning_rows,
            generation,
            state,
            sync_guard,
            handler,
        };
        controls.connect_actions();
        controls.sync_from_state();
        controls
    }

    #[must_use]
    pub fn widget(&self) -> &gtk4::Expander {
        &self.root
    }
    #[must_use]
    pub fn state(&self) -> ProfileControlsState {
        self.state.borrow().clone()
    }
    pub fn set_message_handler(&self, handler: impl Fn(ProfileControlMessage) + 'static) {
        let _ = self.handler.replace(Some(Box::new(handler)));
    }

    #[must_use]
    pub fn set_state(&self, next: ProfileControlsState) -> bool {
        if next.generation() < self.state.borrow().generation() {
            return false;
        }
        self.state.replace(next);
        self.sync_from_state();
        true
    }

    fn connect_actions(&self) {
        for row in &self.rows {
            let role = row.role;
            let selector = row.selector.clone();
            let state = Rc::clone(&self.state);
            let guard = Rc::clone(&self.sync_guard);
            let handler = Rc::clone(&self.handler);
            row.selector.connect_selected_notify(move |_| {
                if guard.get() {
                    return;
                }
                let Ok(selected) = usize::try_from(selector.selected()) else {
                    return;
                };
                let Some(profile) = state
                    .borrow()
                    .role(role)
                    .choices()
                    .get(selected)
                    .map(ProfileChoice::profile)
                else {
                    return;
                };
                emit(
                    &state,
                    &handler,
                    ProfileControlAction::SelectProfile { role, profile },
                );
            });
        }

        let state = Rc::clone(&self.state);
        let guard = Rc::clone(&self.sync_guard);
        let handler = Rc::clone(&self.handler);
        self.intent.connect_selected_notify(move |intent| {
            if !guard.get()
                && let Ok(index) = usize::try_from(intent.selected())
                && let Some(value) = rendering_intent(index)
            {
                emit(&state, &handler, ProfileControlAction::SetIntent(value));
            }
        });
        connect_check(&self.bpc, self, |active| {
            ProfileControlAction::SetBlackPointCompensation(if active {
                BlackPointCompensation::Enabled
            } else {
                BlackPointCompensation::Disabled
            })
        });
        connect_toggle(&self.soft_proof, self, ProfileControlAction::SetSoftProof);
        connect_toggle(
            &self.gamut_warning,
            self,
            ProfileControlAction::SetGamutWarning,
        );
    }

    fn sync_from_state(&self) {
        self.sync_guard.set(true);
        let state = self.state.borrow();
        for row in &self.rows {
            let role = state.role(row.role);
            let labels = role
                .choices()
                .iter()
                .map(ProfileChoice::label)
                .collect::<Vec<_>>();
            let labels = if labels.is_empty() {
                vec![role.status().label()]
            } else {
                labels
            };
            row.selector
                .set_model(Some(&gtk4::StringList::new(&labels)));
            row.selector.set_selected(selected_choice(role));
            row.selector.set_sensitive(!role.choices().is_empty());
            row.status.set_text(role.status().label());
            let accessible = format!("{}: {}", role.role().label(), role.status().label());
            row.status.update_property(&[Property::Label(&accessible)]);
        }
        self.intent.set_selected(intent_index(state.intent()));
        self.bpc
            .set_active(state.black_point_compensation() == BlackPointCompensation::Enabled);
        let display_ready = state.role(ProfileRole::Display).status().is_ready();
        let proof_ready = state.role(ProfileRole::SoftProof).status().is_ready();
        self.soft_proof.set_sensitive(display_ready && proof_ready);
        self.soft_proof.set_active(state.soft_proof_enabled());
        self.gamut_warning.set_sensitive(proof_ready);
        self.gamut_warning.set_active(state.gamut_warning_enabled());
        self.status.set_text(if state.warnings().is_empty() {
            "Color profiles ready"
        } else {
            "Color profile attention required"
        });
        for (index, row) in self.warning_rows.iter().enumerate() {
            if let Some(warning) = state.warnings().get(index) {
                let text = format!("{}: {}", warning.role().label(), warning.message());
                row.set_text(&text);
                row.update_property(&[Property::Label(&text)]);
                row.set_visible(true);
            } else {
                row.set_text("");
                row.set_visible(false);
            }
        }
        self.generation.set_text(&format!(
            "profile decision generation {}",
            state.generation()
        ));
        self.sync_guard.set(false);
    }
}

impl Default for ProfileControls {
    fn default() -> Self {
        Self::new(ProfileControlsState::default())
    }
}
fn profile_row(role: ProfileRole, content: &gtk4::Box) -> ProfileRow {
    let selector = gtk4::DropDown::from_strings(&["not resolved"]);
    setup_accessible(
        &selector,
        PROFILE_CONTROL_WIDGET_IDS[1 + role.index()],
        role.label(),
    );
    append_labeled(content, role.label(), &selector);
    let status = status_label("color-profile-role-status", role.label());
    status.add_css_class("dim-label");
    content.append(&status);
    ProfileRow {
        role,
        selector,
        status,
    }
}
fn append_labeled(content: &gtk4::Box, label: &str, child: &impl IsA<gtk4::Widget>) {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    row.set_hexpand(true);
    let text = gtk4::Label::new(Some(label));
    text.set_halign(gtk4::Align::Start);
    text.set_hexpand(true);
    row.append(&text);
    row.append(child);
    content.append(&row);
}
fn setup_accessible(
    widget: &(impl IsA<gtk4::Widget> + IsA<gtk4::Accessible>),
    id: &str,
    accessible_name: &str,
) {
    widget.set_widget_name(id);
    widget.update_property(&[Property::Label(accessible_name)]);
}
fn status_label(id: &str, accessible_name: &str) -> gtk4::Label {
    let label = gtk4::Label::new(None);
    setup_accessible(&label, id, accessible_name);
    label.set_accessible_role(gtk4::AccessibleRole::Status);
    label.set_halign(gtk4::Align::Start);
    label.set_wrap(true);
    label
}

fn setup_toggle(button: &gtk4::ToggleButton, id: &str, accessible_name: &str) {
    setup_accessible(button, id, accessible_name);
    button.set_accessible_role(gtk4::AccessibleRole::ToggleButton);
    button.set_tooltip_text(Some(accessible_name));
}
fn connect_check(
    button: &gtk4::CheckButton,
    controls: &ProfileControls,
    action: impl Fn(bool) -> ProfileControlAction + 'static,
) {
    let state = Rc::clone(&controls.state);
    let guard = Rc::clone(&controls.sync_guard);
    let handler = Rc::clone(&controls.handler);
    button.connect_toggled(move |button| {
        if !guard.get() {
            emit(&state, &handler, action(button.is_active()));
        }
    });
}
fn connect_toggle(
    button: &gtk4::ToggleButton,
    controls: &ProfileControls,
    action: impl Fn(bool) -> ProfileControlAction + 'static,
) {
    let state = Rc::clone(&controls.state);
    let guard = Rc::clone(&controls.sync_guard);
    let handler = Rc::clone(&controls.handler);
    button.connect_toggled(move |button| {
        if !guard.get() {
            emit(&state, &handler, action(button.is_active()));
        }
    });
}
fn emit(
    state: &Rc<RefCell<ProfileControlsState>>,
    handler: &Rc<RefCell<Option<ProfileMessageHandler>>>,
    action: ProfileControlAction,
) {
    let message = ProfileControlMessage::new(state.borrow().generation(), action);
    if let Some(handler) = handler.borrow().as_ref() {
        handler(message);
    }
}
fn selected_choice(role: &ProfileRoleState) -> u32 {
    role.profile()
        .and_then(|profile| {
            role.choices()
                .iter()
                .position(|choice| choice.profile() == profile)
        })
        .and_then(|index| u32::try_from(index).ok())
        .unwrap_or(0)
}
fn rendering_intent(index: usize) -> Option<RenderingIntent> {
    [
        RenderingIntent::Perceptual,
        RenderingIntent::Relative,
        RenderingIntent::Saturation,
        RenderingIntent::Absolute,
    ]
    .get(index)
    .copied()
}
fn intent_index(intent: RenderingIntent) -> u32 {
    match intent {
        RenderingIntent::Perceptual => 0,
        RenderingIntent::Relative => 1,
        RenderingIntent::Saturation => 2,
        RenderingIntent::Absolute => 3,
    }
}

#[cfg(test)]
#[path = "profile_controls/tests.rs"]
mod tests;
