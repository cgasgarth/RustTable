//! GTK widget implementation for [`super::ProfileControls`].

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::accessible::Property;
use gtk4::prelude::*;

use super::{
    BlackPointCompensation, MAX_PROFILE_WARNINGS, PROFILE_CONTROL_WIDGET_IDS, ProfileChoice,
    ProfileControlAction, ProfileControlMessage, ProfileControlsState, ProfileRole,
    RenderingIntent,
};

type ProfileMessageHandler = Box<dyn Fn(ProfileControlMessage)>;

#[derive(Clone)]
struct ProfileRow {
    role: ProfileRole,
    selector: gtk4::DropDown,
    status: gtk4::Label,
}

/// GTK4 profile selectors, proofing toggles, and bounded diagnostic status.
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
    /// Builds the Darktable-shaped profile group from an already-resolved typed state.
    #[must_use]
    pub fn new(initial: ProfileControlsState) -> Self {
        let state = Rc::new(RefCell::new(initial));
        let sync_guard = Rc::new(Cell::new(false));
        let handler = Rc::new(RefCell::new(None));
        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 3);
        content.set_widget_name("color-profile-controls-content");

        let rows = ProfileRole::ALL
            .into_iter()
            .map(|role| profile_row(role, &content))
            .collect::<Vec<_>>();
        let intent = gtk4::DropDown::from_strings(&[
            "perceptual",
            "relative colorimetric",
            "saturation",
            "absolute colorimetric",
        ]);
        intent.set_widget_name(PROFILE_CONTROL_WIDGET_IDS[6]);
        intent.set_accessible_role(gtk4::AccessibleRole::ComboBox);
        intent.update_property(&[Property::Label("Rendering intent")]);
        append_labeled(
            &content,
            "rendering intent",
            &intent,
            "color-profile-intent-row",
        );

        let bpc = gtk4::CheckButton::with_label("black-point compensation");
        bpc.set_widget_name(PROFILE_CONTROL_WIDGET_IDS[7]);
        bpc.set_accessible_role(gtk4::AccessibleRole::Checkbox);
        bpc.update_property(&[Property::Label("Black-point compensation")]);
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

        let status = gtk4::Label::new(None);
        status.set_widget_name(PROFILE_CONTROL_WIDGET_IDS[13]);
        status.set_halign(gtk4::Align::Start);
        status.set_wrap(true);
        status.set_accessible_role(gtk4::AccessibleRole::Status);
        status.update_property(&[Property::Label("Color profile status")]);
        content.append(&status);

        let warning_rows = (0..MAX_PROFILE_WARNINGS)
            .map(|index| {
                let row = gtk4::Label::new(None);
                row.set_widget_name(PROFILE_CONTROL_WIDGET_IDS[10 + index]);
                row.set_halign(gtk4::Align::Start);
                row.set_wrap(true);
                row.add_css_class("warning");
                row.set_accessible_role(gtk4::AccessibleRole::Status);
                content.append(&row);
                row
            })
            .collect::<Vec<_>>();
        let generation = gtk4::Label::new(None);
        generation.set_widget_name(PROFILE_CONTROL_WIDGET_IDS[14]);
        generation.add_css_class("dim-label");
        generation.set_accessible_role(gtk4::AccessibleRole::Status);
        generation.update_property(&[Property::Label("Profile decision generation")]);
        content.append(&generation);

        let root = gtk4::Expander::builder()
            .label("color profiles")
            .expanded(true)
            .child(&content)
            .build();
        root.set_widget_name(PROFILE_CONTROL_WIDGET_IDS[0]);
        root.set_hexpand(true);
        root.set_accessible_role(gtk4::AccessibleRole::Group);
        root.update_property(&[Property::Label("Color profiles and soft proofing")]);

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

    /// Installs the application/controller receiver for typed profile intents.
    pub fn set_message_handler(&self, handler: impl Fn(ProfileControlMessage) + 'static) {
        self.handler.replace(Some(Box::new(handler)));
    }

    /// Reprojects a decision only when it is not older than the visible decision.
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
                let Some(index) = usize::try_from(selector.selected()).ok() else {
                    return;
                };
                let Some(profile) = state
                    .borrow()
                    .role(role)
                    .choices()
                    .get(index)
                    .map(ProfileChoice::profile)
                else {
                    return;
                };
                let message = ProfileControlMessage::new(
                    state.borrow().generation(),
                    ProfileControlAction::SelectProfile { role, profile },
                );
                if let Some(handler) = handler.borrow().as_ref() {
                    handler(message);
                }
            });
        }

        let state = Rc::clone(&self.state);
        let guard = Rc::clone(&self.sync_guard);
        let handler = Rc::clone(&self.handler);
        self.intent.connect_selected_notify(move |intent| {
            if guard.get() {
                return;
            }
            let Some(index) = usize::try_from(intent.selected()).ok() else {
                return;
            };
            let Some(value) = rendering_intent(index) else {
                return;
            };
            emit(&state, &handler, ProfileControlAction::SetIntent(value));
        });

        connect_check(
            &self.bpc,
            &self.state,
            &self.sync_guard,
            &self.handler,
            |active| {
                ProfileControlAction::SetBlackPointCompensation(if active {
                    BlackPointCompensation::Enabled
                } else {
                    BlackPointCompensation::Disabled
                })
            },
        );
        connect_toggle(
            &self.soft_proof,
            &self.state,
            &self.sync_guard,
            &self.handler,
            ProfileControlAction::SetSoftProof,
        );
        connect_toggle(
            &self.gamut_warning,
            &self.state,
            &self.sync_guard,
            &self.handler,
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
                .map(|choice| choice.label())
                .collect::<Vec<_>>();
            let labels = if labels.is_empty() {
                vec![role.status().label()]
            } else {
                labels
            };
            row.selector
                .set_model(Some(&gtk4::StringList::new(&labels)));
            row.selector.set_selected(
                role.profile()
                    .and_then(|profile| {
                        role.choices()
                            .iter()
                            .position(|choice| choice.profile() == profile)
                    })
                    .and_then(|index| u32::try_from(index).ok())
                    .unwrap_or(0),
            );
            row.selector.set_sensitive(!role.choices().is_empty());
            row.status.set_text(role.status().label());
            row.status.set_tooltip_text(Some(&format!(
                "{}: {}",
                role.role().label(),
                role.status().label()
            )));
        }
        self.intent.set_selected(intent_index(state.intent()));
        self.bpc
            .set_active(state.black_point_compensation() == BlackPointCompensation::Enabled);
        let proof_ready = state.role(ProfileRole::SoftProof).status().is_ready();
        self.soft_proof
            .set_sensitive(proof_ready && state.role(ProfileRole::Display).status().is_ready());
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
                row.set_text(&format!(
                    "{}: {}",
                    warning.role().label(),
                    warning.message()
                ));
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
    selector.set_widget_name(PROFILE_CONTROL_WIDGET_IDS[1 + role.index()]);
    selector.set_accessible_role(gtk4::AccessibleRole::ComboBox);
    selector.update_property(&[Property::Label(role.label())]);
    let status = gtk4::Label::new(None);
    status.set_halign(gtk4::Align::Start);
    status.add_css_class("dim-label");
    status.set_accessible_role(gtk4::AccessibleRole::Status);
    append_labeled(
        content,
        role.label(),
        &selector,
        &format!("{}-row", role.label()),
    );
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    row.set_widget_name(&format!("{}-status-row", role.label()));
    row.append(&status);
    content.append(&row);
    ProfileRow {
        role,
        selector,
        status,
    }
}

fn append_labeled(content: &gtk4::Box, label: &str, child: &impl IsA<gtk4::Widget>, id: &str) {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    row.set_widget_name(id);
    row.set_hexpand(true);
    let text = gtk4::Label::new(Some(label));
    text.set_halign(gtk4::Align::Start);
    text.set_hexpand(true);
    row.append(&text);
    row.append(child);
    content.append(&row);
}

fn setup_toggle(button: &gtk4::ToggleButton, id: &str, accessible_name: &str) {
    button.set_widget_name(id);
    button.set_focus_on_click(false);
    button.set_accessible_role(gtk4::AccessibleRole::ToggleButton);
    button.set_tooltip_text(Some(accessible_name));
    button.update_property(&[Property::Label(accessible_name)]);
}

fn connect_check(
    button: &gtk4::CheckButton,
    state: &Rc<RefCell<ProfileControlsState>>,
    guard: &Rc<Cell<bool>>,
    handler: &Rc<RefCell<Option<ProfileMessageHandler>>>,
    action: impl Fn(bool) -> ProfileControlAction + 'static,
) {
    let state = Rc::clone(state);
    let guard = Rc::clone(guard);
    let handler = Rc::clone(handler);
    button.connect_toggled(move |button| {
        if !guard.get() {
            emit(&state, &handler, action(button.is_active()));
        }
    });
}

fn connect_toggle(
    button: &gtk4::ToggleButton,
    state: &Rc<RefCell<ProfileControlsState>>,
    guard: &Rc<Cell<bool>>,
    handler: &Rc<RefCell<Option<ProfileMessageHandler>>>,
    action: impl Fn(bool) -> ProfileControlAction + 'static,
) {
    let state = Rc::clone(state);
    let guard = Rc::clone(guard);
    let handler = Rc::clone(handler);
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
