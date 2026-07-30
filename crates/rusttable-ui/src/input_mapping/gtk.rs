//! Native GTK4 realization of the shortcut/device editor.
//!
//! Darktable behavior mapped here comes from `src/gui/accelerators.c` (the
//! action/shortcut tree, capture, delete/default semantics),
//! `src/gui/preferences.c` (preferences placement and import/export affordance),
//! `src/libs/tools/midi.c` (source/channel/control descriptors), and
//! `src/libs/tools/gamepad.c` (source capability and unavailable-device rows).
//! GTK4 replaces the upstream mutable tree views and backend callbacks; this
//! module receives only privacy-safe typed snapshots.

use super::default_snapshot;
use super::profile::{LocalProfileIo, ProfileIo};
use super::state::EditorState;
use super::types::{
    ActionId, Binding, EditorMessage, EditorStatus, EditorView, KeyChord, KeyModifier, LearnTarget,
    MappingSnapshot, ResetScope,
};
use gtk4::gdk;
use gtk4::glib::{self, ControlFlow, Propagation};
use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

#[derive(Clone)]
struct EditorWidgets {
    window: gtk4::Window,
    search: gtk4::SearchEntry,
    stack: gtk4::Stack,
    actions: gtk4::ListBox,
    devices: gtk4::ListBox,
    details: gtk4::Box,
    status: gtk4::Label,
    action_ids: Rc<RefCell<Vec<ActionId>>>,
    device_aliases: Rc<RefCell<Vec<String>>>,
}

/// Preferences window for action-centric and device-centric mapping editing.
///
/// The window owns only GTK widgets and an [`EditorState`].  It does not own
/// device handles, configuration paths, or an action dispatcher.
#[derive(Clone)]
pub struct InputMappingEditor {
    widgets: EditorWidgets,
    state: Rc<RefCell<EditorState>>,
    profile_io: Rc<dyn ProfileIo>,
}

impl InputMappingEditor {
    /// Builds the editor using the deterministic registry fixture.
    #[must_use]
    pub fn new(application: &gtk4::Application) -> Self {
        Self::from_snapshot(application, default_snapshot())
    }

    /// Builds the editor over a snapshot supplied by the input service.
    #[must_use]
    pub fn from_snapshot(application: &gtk4::Application, snapshot: MappingSnapshot) -> Self {
        Self::with_profile_io(application, snapshot, Rc::new(LocalProfileIo))
    }

    /// Builds the editor with an application-owned profile persistence port.
    #[must_use]
    pub fn with_profile_io(
        application: &gtk4::Application,
        snapshot: MappingSnapshot,
        profile_io: Rc<dyn ProfileIo>,
    ) -> Self {
        let window = gtk4::Window::builder()
            .application(application)
            .title("Shortcuts & input")
            .default_width(1_040)
            .default_height(680)
            .hide_on_close(true)
            .build();
        window.set_widget_name("input-mapping-editor");

        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
        root.set_margin_start(16);
        root.set_margin_end(16);
        root.set_margin_top(12);
        root.set_margin_bottom(12);

        let heading = gtk4::Label::new(Some("Shortcuts & input"));
        heading.set_halign(gtk4::Align::Start);
        heading.add_css_class("title-2");

        let search = gtk4::SearchEntry::new();
        search.set_widget_name("input-mapping-search");
        search.set_placeholder_text(Some("Search actions, categories, devices, or stable IDs"));
        search.set_hexpand(true);
        search.set_accessible_role(gtk4::AccessibleRole::SearchBox);

        let stack = gtk4::Stack::builder()
            .transition_type(gtk4::StackTransitionType::Crossfade)
            .vexpand(true)
            .build();
        let actions = gtk4::ListBox::new();
        actions.set_selection_mode(gtk4::SelectionMode::Single);
        actions.set_widget_name("input-mapping-actions");
        actions.set_vexpand(true);
        let devices = gtk4::ListBox::new();
        devices.set_selection_mode(gtk4::SelectionMode::Single);
        devices.set_widget_name("input-mapping-devices");
        devices.set_vexpand(true);
        stack.add_titled(&actions, Some("actions"), "Actions");
        stack.add_titled(&devices, Some("devices"), "Devices");
        let switcher = gtk4::StackSwitcher::builder().stack(&stack).build();

        let details = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        details.set_widget_name("input-mapping-details");
        details.set_margin_start(12);
        details.set_hexpand(true);
        details.set_vexpand(true);

        let split = gtk4::Paned::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .start_child(&stack)
            .end_child(&details)
            .resize_start_child(true)
            .shrink_start_child(false)
            .position(360)
            .build();

        let status = gtk4::Label::new(None);
        status.set_widget_name("input-mapping-status");
        status.set_halign(gtk4::Align::Start);
        status.set_wrap(true);

        let actions_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let import = gtk4::Button::with_label("Import profile…");
        let export = gtk4::Button::with_label("Export profile…");
        let revert = gtk4::Button::with_label("Revert");
        let reset = gtk4::Button::with_label("Reset all");
        let apply = gtk4::Button::with_label("Apply");
        apply.add_css_class("suggested-action");
        actions_row.append(&import);
        actions_row.append(&export);
        actions_row.append(&revert);
        actions_row.append(&reset);
        actions_row.append(&apply);

        root.append(&heading);
        root.append(&search);
        root.append(&switcher);
        root.append(&split);
        root.append(&status);
        root.append(&actions_row);
        window.set_child(Some(&root));

        let state = Rc::new(RefCell::new(EditorState::new(snapshot)));
        let widgets = EditorWidgets {
            window,
            search,
            stack,
            actions,
            devices,
            details,
            status,
            action_ids: Rc::new(RefCell::new(Vec::new())),
            device_aliases: Rc::new(RefCell::new(Vec::new())),
        };
        let editor = Self {
            widgets,
            state,
            profile_io,
        };
        editor.connect_actions(&import, &export, &revert, &reset, &apply);
        editor.connect_key_capture();
        editor.connect_learn_timer();
        editor.render();
        editor
    }

    /// Presents the preferences window and focuses the search field.
    pub fn present(&self) {
        self.widgets.window.present();
        self.widgets.search.grab_focus();
    }

    /// Returns the preferences window for transient-parent integration.
    #[must_use]
    pub fn window(&self) -> &gtk4::Window {
        &self.widgets.window
    }

    fn connect_actions(
        &self,
        import: &gtk4::Button,
        export: &gtk4::Button,
        revert: &gtk4::Button,
        reset: &gtk4::Button,
        apply: &gtk4::Button,
    ) {
        let state = Rc::clone(&self.state);
        let widgets = self.widgets.clone();
        self.widgets.search.connect_search_changed(move |entry| {
            state
                .borrow_mut()
                .update(EditorMessage::SetSearch(entry.text().to_string()))
                .ok();
            render(&widgets, &state);
        });

        let state = Rc::clone(&self.state);
        let widgets = self.widgets.clone();
        self.widgets
            .stack
            .connect_visible_child_notify(move |stack| {
                let view = if stack.visible_child_name().as_deref() == Some("devices") {
                    EditorView::Devices
                } else {
                    EditorView::Actions
                };
                state.borrow_mut().update(EditorMessage::SetView(view)).ok();
                render(&widgets, &state);
            });

        let state = Rc::clone(&self.state);
        let widgets = self.widgets.clone();
        self.widgets.actions.connect_row_selected(move |_, row| {
            if let Some(row) = row
                && let Ok(index) = usize::try_from(row.index())
                && let Some(action_id) = widgets.action_ids.borrow().get(index)
            {
                state
                    .borrow_mut()
                    .update(EditorMessage::SelectAction(action_id.clone()))
                    .ok();
                render(&widgets, &state);
            }
        });

        let state = Rc::clone(&self.state);
        let widgets = self.widgets.clone();
        self.widgets.devices.connect_row_selected(move |_, row| {
            if let Some(row) = row
                && let Ok(index) = usize::try_from(row.index())
                && let Some(alias) = widgets.device_aliases.borrow().get(index)
            {
                state
                    .borrow_mut()
                    .update(EditorMessage::SelectDevice(alias.clone()))
                    .ok();
                render(&widgets, &state);
            }
        });

        let state = Rc::clone(&self.state);
        let widgets = self.widgets.clone();
        revert.connect_clicked(move |_| {
            state.borrow_mut().update(EditorMessage::Revert).ok();
            render(&widgets, &state);
        });

        let state = Rc::clone(&self.state);
        let widgets = self.widgets.clone();
        reset.connect_clicked(move |_| {
            state
                .borrow_mut()
                .update(EditorMessage::Reset(ResetScope::All))
                .ok();
            render(&widgets, &state);
        });

        let state = Rc::clone(&self.state);
        let widgets = self.widgets.clone();
        apply.connect_clicked(move |_| {
            let generation = state.borrow().snapshot().generation;
            let result = state.borrow_mut().update(EditorMessage::Apply {
                live_generation: generation,
            });
            if let Err(error) = result {
                state.borrow_mut().status = EditorStatus::ValidationError(error.to_string());
            }
            render(&widgets, &state);
        });

        self.connect_profile_io(import, export);
    }

    fn connect_profile_io(&self, import: &gtk4::Button, export: &gtk4::Button) {
        let state = Rc::clone(&self.state);
        let widgets = self.widgets.clone();
        let window = self.widgets.window.clone();
        let profile_io = Rc::clone(&self.profile_io);
        import.connect_clicked(move |_| {
            let dialog = gtk4::FileDialog::builder()
                .title("Import mapping profile")
                .accept_label("Import")
                .modal(true)
                .build();
            let state = Rc::clone(&state);
            let widgets = widgets.clone();
            let profile_io = Rc::clone(&profile_io);
            dialog.open(
                Some(&window),
                None::<&gtk4::gio::Cancellable>,
                move |result| {
                    let Ok(file) = result else { return };
                    let Some(path) = file.path() else { return };
                    match profile_io.load(path.as_path()) {
                        Ok(profile) => state.borrow_mut().import_profile(profile),
                        Err(error) => {
                            state.borrow_mut().status =
                                EditorStatus::ValidationError(error.to_string());
                        }
                    }
                    render(&widgets, &state);
                },
            );
        });

        let state = Rc::clone(&self.state);
        let widgets = self.widgets.clone();
        let window = self.widgets.window.clone();
        let profile_io = Rc::clone(&self.profile_io);
        export.connect_clicked(move |_| {
            let dialog = gtk4::FileDialog::builder()
                .title("Export mapping profile")
                .accept_label("Export")
                .modal(true)
                .build();
            let state = Rc::clone(&state);
            let widgets = widgets.clone();
            let profile_io = Rc::clone(&profile_io);
            dialog.save(
                Some(&window),
                None::<&gtk4::gio::Cancellable>,
                move |result| {
                    let Ok(file) = result else { return };
                    let Some(path) = file.path() else { return };
                    let profile = state.borrow().export_profile("RustTable mappings");
                    match profile_io.save(path.as_path(), &profile) {
                        Ok(()) => state.borrow_mut().status = EditorStatus::Dirty,
                        Err(error) => {
                            state.borrow_mut().status =
                                EditorStatus::ValidationError(error.to_string());
                        }
                    }
                    render(&widgets, &state);
                },
            );
        });
    }

    fn connect_key_capture(&self) {
        let controller = gtk4::EventControllerKey::new();
        let state = Rc::clone(&self.state);
        let widgets = self.widgets.clone();
        controller.connect_key_pressed(move |_, key, _, modifiers| {
            if state.borrow().learn != Some(LearnTarget::Keyboard) {
                return Propagation::Proceed;
            }
            let key_name = key
                .name()
                .map_or_else(|| "Key".to_owned(), |name| name.to_string());
            let mut normalized = Vec::new();
            if modifiers.contains(gdk::ModifierType::SHIFT_MASK) {
                normalized.push(KeyModifier::Shift);
            }
            if modifiers.contains(gdk::ModifierType::CONTROL_MASK) {
                normalized.push(KeyModifier::Control);
            }
            if modifiers.contains(gdk::ModifierType::ALT_MASK) {
                normalized.push(KeyModifier::Alt);
            }
            if modifiers.contains(gdk::ModifierType::SUPER_MASK) {
                normalized.push(KeyModifier::Super);
            }
            state
                .borrow_mut()
                .update(EditorMessage::CaptureKeyboard(KeyChord::new(
                    key_name, normalized,
                )))
                .ok();
            render(&widgets, &state);
            Propagation::Stop
        });
        self.widgets.window.add_controller(controller);
    }

    fn connect_learn_timer(&self) {
        let state = Rc::clone(&self.state);
        let widgets = self.widgets.clone();
        glib::timeout_add_local(Duration::from_secs(1), move || {
            if state.borrow().learn.is_some() {
                state.borrow_mut().update(EditorMessage::LearnTick).ok();
                render(&widgets, &state);
            }
            ControlFlow::Continue
        });
    }

    fn render(&self) {
        render(&self.widgets, &self.state);
    }
}

fn render(widgets: &EditorWidgets, state: &Rc<RefCell<EditorState>>) {
    let state_ref = state.borrow();
    widgets.search.set_text(&state_ref.search);
    widgets
        .stack
        .set_visible_child_name(if state_ref.view == EditorView::Devices {
            "devices"
        } else {
            "actions"
        });
    widgets.status.set_text(&status_text(&state_ref.status));

    clear_listbox(&widgets.actions);
    let actions = state_ref.visible_actions();
    widgets.action_ids.borrow_mut().clear();
    for action in actions {
        widgets.action_ids.borrow_mut().push(action.id.clone());
        let binding_count = state_ref.bindings_for(&action.id).len();
        let conflicts = state_ref
            .conflicts()
            .iter()
            .filter(|conflict| {
                state_ref.bindings_for(&action.id).iter().any(|binding| {
                    binding.id == conflict.left_binding_id
                        || binding.id == conflict.right_binding_id
                })
            })
            .count();
        let row = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        row.set_margin_start(8);
        row.set_margin_end(8);
        row.set_margin_top(6);
        row.set_margin_bottom(6);
        let label = gtk4::Label::new(Some(&action.label));
        label.set_halign(gtk4::Align::Start);
        let detail = format!(
            "{} · {} binding(s){}",
            action.category,
            binding_count,
            if conflicts == 0 { "" } else { " · conflict" }
        );
        let secondary = gtk4::Label::new(Some(&detail));
        secondary.set_halign(gtk4::Align::Start);
        secondary.add_css_class(if conflicts == 0 { "dim-label" } else { "error" });
        row.append(&label);
        row.append(&secondary);
        widgets.actions.append(&row);
    }

    clear_listbox(&widgets.devices);
    widgets.device_aliases.borrow_mut().clear();
    for device in state_ref.visible_devices() {
        widgets
            .device_aliases
            .borrow_mut()
            .push(device.alias.clone());
        let row = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        row.set_margin_start(8);
        row.set_margin_end(8);
        row.set_margin_top(6);
        row.set_margin_bottom(6);
        let label = gtk4::Label::new(Some(&device.label));
        label.set_halign(gtk4::Align::Start);
        let availability = if device.available {
            "Available"
        } else {
            "Offline; mappings remain editable"
        };
        let secondary =
            gtk4::Label::new(Some(&format!("{} · {}", device.kind.label(), availability)));
        secondary.set_halign(gtk4::Align::Start);
        secondary.add_css_class(if device.available {
            "dim-label"
        } else {
            "warning"
        });
        row.append(&label);
        row.append(&secondary);
        widgets.devices.append(&row);
    }

    clear_box(&widgets.details);
    match state_ref.view {
        EditorView::Actions => render_action_details(widgets, &state_ref, state),
        EditorView::Devices => render_device_details(widgets, &state_ref, state),
    }
}

fn render_action_details(
    widgets: &EditorWidgets,
    state: &EditorState,
    shared: &Rc<RefCell<EditorState>>,
) {
    let Some(action_id) = state.selected_action.as_ref() else {
        widgets.details.append(&gtk4::Label::new(Some(
            "Select an action to edit its mappings.",
        )));
        return;
    };
    let Some(action) = state
        .snapshot()
        .actions
        .iter()
        .find(|action| &action.id == action_id)
    else {
        widgets.details.append(&gtk4::Label::new(Some(
            "The selected action is no longer available.",
        )));
        return;
    };
    append_heading(&widgets.details, &action.label);
    append_text(
        &widgets.details,
        &format!("{} · {}", action.category, action.id),
    );
    append_text(
        &widgets.details,
        &format!(
            "Contexts: {}",
            action
                .contexts
                .iter()
                .map(|context| context.label())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    );
    if !action.available {
        append_text(&widgets.details, "Unavailable in the current runtime");
    }
    if let Some(parameter) = action.parameter.as_ref() {
        append_text(
            &widgets.details,
            &format!(
                "Continuous parameter: {:.2} … {:.2}, step {:.3}",
                parameter.min, parameter.max, parameter.step
            ),
        );
    }

    for binding in state.bindings_for(action_id) {
        binding_row(widgets, state, shared, binding);
    }

    let learn_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    for (label, target) in [
        ("Learn keyboard", LearnTarget::Keyboard),
        ("Learn pointer", LearnTarget::Pointer),
        ("Learn tablet", LearnTarget::Tablet),
        ("Learn MIDI", LearnTarget::Midi),
        ("Learn gamepad", LearnTarget::Gamepad),
    ] {
        let button = gtk4::Button::with_label(label);
        let shared = Rc::clone(shared);
        let widgets = widgets.clone();
        button.connect_clicked(move |_| {
            shared
                .borrow_mut()
                .update(EditorMessage::BeginLearn(target))
                .ok();
            render(&widgets, &shared);
        });
        learn_row.append(&button);
    }
    widgets.details.append(&learn_row);

    let reset = gtk4::Button::with_label("Reset this action");
    let shared = Rc::clone(shared);
    let widgets_clone = widgets.clone();
    reset.connect_clicked(move |_| {
        shared
            .borrow_mut()
            .update(EditorMessage::Reset(ResetScope::Action))
            .ok();
        render(&widgets_clone, &shared);
    });
    widgets.details.append(&reset);
}

fn binding_row(
    widgets: &EditorWidgets,
    state: &EditorState,
    shared: &Rc<RefCell<EditorState>>,
    binding: &Binding,
) {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    row.set_margin_top(3);
    row.set_margin_bottom(3);
    let text = gtk4::Label::new(Some(&format!(
        "{} · {} · {}",
        binding.source.display(),
        binding.context.label(),
        binding.device_alias
    )));
    text.set_halign(gtk4::Align::Start);
    text.set_hexpand(true);
    text.set_wrap(true);
    let enabled = gtk4::Switch::new();
    enabled.set_active(binding.enabled);
    enabled.set_tooltip_text(Some("Enable binding"));
    let binding_id = binding.id.clone();
    let shared_switch = Rc::clone(shared);
    let widgets_switch = widgets.clone();
    enabled.connect_active_notify(move |switch| {
        shared_switch
            .borrow_mut()
            .update(EditorMessage::ToggleBinding {
                binding_id: binding_id.clone(),
                enabled: switch.is_active(),
            })
            .ok();
        render(&widgets_switch, &shared_switch);
    });
    let test_button = gtk4::Button::with_label("Test binding");
    let binding_id = binding.id.clone();
    let shared_test = Rc::clone(shared);
    let widgets_test = widgets.clone();
    test_button.connect_clicked(move |_| {
        shared_test
            .borrow_mut()
            .update(EditorMessage::TestBinding(binding_id.clone()))
            .ok();
        render(&widgets_test, &shared_test);
    });
    let remove = gtk4::Button::with_label(if binding.built_in {
        "Disable"
    } else {
        "Remove"
    });
    let binding_id = binding.id.clone();
    let shared_remove = Rc::clone(shared);
    let widgets_remove = widgets.clone();
    remove.connect_clicked(move |_| {
        let result = shared_remove
            .borrow_mut()
            .update(EditorMessage::RemoveBinding(binding_id.clone()));
        if let Err(error) = result {
            shared_remove.borrow_mut().status = EditorStatus::ValidationError(error.to_string());
        }
        render(&widgets_remove, &shared_remove);
    });
    row.append(&text);
    row.append(&enabled);
    row.append(&test_button);
    row.append(&remove);
    widgets.details.append(&row);

    if let Some(conflict) = state.conflicts().into_iter().find(|conflict| {
        conflict.left_binding_id == binding.id || conflict.right_binding_id == binding.id
    }) {
        let warning = gtk4::Label::new(Some(&conflict.explanation));
        warning.set_halign(gtk4::Align::Start);
        warning.set_wrap(true);
        warning.add_css_class(if conflict.blocks_apply() {
            "error"
        } else {
            "warning"
        });
        widgets.details.append(&warning);
    }
}

fn render_device_details(
    widgets: &EditorWidgets,
    state: &EditorState,
    shared: &Rc<RefCell<EditorState>>,
) {
    let Some(alias) = state.selected_device.as_ref() else {
        widgets.details.append(&gtk4::Label::new(Some(
            "Select a device to inspect its mappings.",
        )));
        return;
    };
    let Some(device) = state
        .snapshot()
        .devices
        .iter()
        .find(|device| &device.alias == alias)
    else {
        return;
    };
    append_heading(&widgets.details, &device.label);
    append_text(
        &widgets.details,
        &format!("{} · alias {}", device.kind.label(), device.alias),
    );
    append_text(
        &widgets.details,
        if device.available {
            "Available"
        } else {
            "Offline; stored capability descriptors remain editable"
        },
    );
    append_text(
        &widgets.details,
        &format!("Capabilities: {}", device.capabilities.join(", ")),
    );
    let mappings: Vec<_> = state
        .snapshot()
        .bindings
        .iter()
        .filter(|binding| binding.device_alias == device.alias)
        .collect();
    append_text(&widgets.details, &format!("{} mapping(s)", mappings.len()));
    if !mappings.is_empty() {
        for binding in mappings {
            append_text(
                &widgets.details,
                &format!(
                    "{} → {} ({})",
                    binding.source.display(),
                    binding.action_id,
                    binding.context.label()
                ),
            );
        }
    }
    let reset = gtk4::Button::with_label("Reset this device");
    let shared = Rc::clone(shared);
    let widgets_clone = widgets.clone();
    let kind = device.kind;
    reset.connect_clicked(move |_| {
        shared
            .borrow_mut()
            .update(EditorMessage::Reset(ResetScope::Device(kind)))
            .ok();
        render(&widgets_clone, &shared);
    });
    widgets.details.append(&reset);
}

fn append_heading(container: &gtk4::Box, text: &str) {
    let label = gtk4::Label::new(Some(text));
    label.set_halign(gtk4::Align::Start);
    label.add_css_class("title-3");
    container.append(&label);
}

fn append_text(container: &gtk4::Box, text: &str) {
    let label = gtk4::Label::new(Some(text));
    label.set_halign(gtk4::Align::Start);
    label.set_wrap(true);
    container.append(&label);
}

fn status_text(status: &EditorStatus) -> String {
    match status {
        EditorStatus::Clean => "No unsaved mapping changes.".to_owned(),
        EditorStatus::Dirty => {
            "Unsaved changes; Apply validates and commits one mapping generation.".to_owned()
        }
        EditorStatus::Learning(target) => format!(
            "Learning {} input. Press one input now; captured input will not execute the action. Timeout in 15 seconds of inactivity.",
            target_label(*target)
        ),
        EditorStatus::LearnCaptured => "Input captured. Review the binding, then Apply.".to_owned(),
        EditorStatus::LearnTimedOut => {
            "Learn mode timed out after 15 seconds; no binding was captured.".to_owned()
        }
        EditorStatus::Testing => {
            "Test binding mode is active; observations are preview-only.".to_owned()
        }
        EditorStatus::TestPreview(value) => value.clone(),
        EditorStatus::Applied(generation) => format!("Applied mapping generation {generation}."),
        EditorStatus::StaleGeneration => {
            "Mappings changed elsewhere. Reload or Revert before applying.".to_owned()
        }
        EditorStatus::ValidationError(error) => format!("Cannot apply: {error}"),
        EditorStatus::Imported { changed, unknown } => {
            format!("Imported {changed} mapping(s); {unknown} unknown record(s) retained inactive.")
        }
    }
}

const fn target_label(target: LearnTarget) -> &'static str {
    match target {
        LearnTarget::Keyboard => "keyboard",
        LearnTarget::Pointer => "pointer",
        LearnTarget::Tablet => "tablet",
        LearnTarget::Midi => "MIDI",
        LearnTarget::Gamepad => "gamepad",
    }
}

fn clear_box(container: &gtk4::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn clear_listbox(container: &gtk4::ListBox) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}
