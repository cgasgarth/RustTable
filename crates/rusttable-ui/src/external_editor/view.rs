//! GTK4 projection for the typed external-editor state.

#![allow(clippy::missing_panics_doc)]

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::accessible::Property;
use gtk4::prelude::*;

use super::model::{
    ArgumentRow, ExternalEditorAction, ExternalEditorJob, ExternalEditorPreset,
    ExternalEditorViewModel, InterchangeMode, InvocationReview, MetadataPolicy, Placeholder,
    PresetId, SendToEditorRequest, TiffBitDepth,
};
use crate::presentation::PresentationText;

/// Stable focus order used by the panel and its keyboard/a11y fixture.
pub const EXTERNAL_EDITOR_FOCUS_ORDER: [&str; 9] = [
    "external-editor-preset",
    "external-editor-name",
    "external-editor-executable",
    "external-editor-interchange",
    "external-editor-arguments",
    "external-editor-profile",
    "external-editor-destination",
    "external-editor-test",
    "external-editor-send",
];

type ActionHandler = Box<dyn Fn(ExternalEditorAction) + 'static>;

/// Darktable-shaped right-panel module for defining, qualifying, and running editor presets.
#[derive(Clone)]
pub struct ExternalEditorPanel {
    root: gtk4::Box,
    preset_dropdown: gtk4::DropDown,
    name: gtk4::Entry,
    executable: gtk4::Button,
    executable_status: gtk4::Label,
    interchange: gtk4::DropDown,
    arguments: gtk4::ListBox,
    add_literal: gtk4::Button,
    add_placeholder: gtk4::Button,
    profile: gtk4::Entry,
    destination: gtk4::Entry,
    bit_depth: gtk4::DropDown,
    metadata: gtk4::DropDown,
    xmp: gtk4::CheckButton,
    add_to_catalog: gtk4::CheckButton,
    group_with_source: gtk4::CheckButton,
    save: gtk4::Button,
    test: gtk4::Button,
    send: gtk4::Button,
    confirm: gtk4::Button,
    cancel: gtk4::Button,
    reconcile: gtk4::Button,
    selection: gtk4::Label,
    review: gtk4::Label,
    progress: gtk4::ProgressBar,
    status: gtk4::Label,
    current_preset: Rc<RefCell<Option<ExternalEditorPreset>>>,
    preset_ids: Rc<RefCell<Vec<PresetId>>>,
    draft_arguments: Rc<RefCell<Vec<ArgumentRow>>>,
    current_review: Rc<RefCell<Option<InvocationReview>>>,
    current_job: Rc<RefCell<Option<ExternalEditorJob>>>,
}

impl ExternalEditorPanel {
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn new() -> Self {
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        root.set_widget_name("external-editors");
        root.set_margin_top(6);
        root.set_margin_bottom(6);
        root.set_margin_start(6);
        root.set_margin_end(6);
        root.set_accessible_role(gtk4::AccessibleRole::Region);
        root.update_property(&[Property::Label("External editors")]);

        let title = gtk4::Label::new(Some("External editors"));
        title.set_halign(gtk4::Align::Start);
        title.add_css_class("title-4");
        root.append(&title);

        let preset_dropdown = gtk4::DropDown::from_strings(&["No presets"]);
        preset_dropdown.set_widget_name("external-editor-preset");
        preset_dropdown.set_hexpand(true);
        preset_dropdown.set_tooltip_text(Some("Choose a qualified editor preset"));
        root.append(&labeled_row("Preset", &preset_dropdown));

        let name = gtk4::Entry::new();
        name.set_widget_name("external-editor-name");
        name.set_placeholder_text(Some("Preset name"));
        root.append(&labeled_row("Name", &name));

        let executable = gtk4::Button::with_label("Choose executable…");
        executable.set_widget_name("external-editor-executable");
        executable.set_focusable(true);
        let executable_status = gtk4::Label::new(Some("No executable approved"));
        executable_status.set_halign(gtk4::Align::Start);
        executable_status.add_css_class("dim-label");
        root.append(&labeled_row("Executable", &executable));
        root.append(&indent(&executable_status));

        let interchange = gtk4::DropDown::from_strings(&[
            InterchangeMode::InPlaceTiff.label(),
            InterchangeMode::SeparateOutputTiff.label(),
        ]);
        interchange.set_widget_name("external-editor-interchange");
        root.append(&labeled_row("Interchange", &interchange));

        let arguments = gtk4::ListBox::new();
        arguments.set_widget_name("external-editor-arguments");
        arguments.set_selection_mode(gtk4::SelectionMode::None);
        arguments.set_accessible_role(gtk4::AccessibleRole::List);
        let argument_box = gtk4::Box::new(gtk4::Orientation::Vertical, 3);
        argument_box.append(&arguments);
        let argument_actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
        let add_literal = gtk4::Button::with_label("+ literal");
        add_literal.set_widget_name("external-editor-add-literal");
        let add_placeholder = gtk4::Button::with_label("+ placeholder");
        add_placeholder.set_widget_name("external-editor-add-placeholder");
        argument_actions.append(&add_literal);
        argument_actions.append(&add_placeholder);
        argument_box.append(&argument_actions);
        root.append(&labeled_row("Arguments", &argument_box));

        let profile = gtk4::Entry::new();
        profile.set_widget_name("external-editor-profile");
        profile.set_text("sRGB");
        root.append(&labeled_row("Profile", &profile));

        let destination = gtk4::Entry::new();
        destination.set_widget_name("external-editor-destination");
        destination.set_placeholder_text(Some("Destination template"));
        root.append(&labeled_row("Destination", &destination));

        let bit_depth = gtk4::DropDown::from_strings(&[
            TiffBitDepth::Sixteen.label(),
            TiffBitDepth::Float32.label(),
        ]);
        bit_depth.set_widget_name("external-editor-bit-depth");
        root.append(&labeled_row("TIFF depth", &bit_depth));
        let metadata = gtk4::DropDown::from_strings(&["Preserve metadata", "Minimal metadata"]);
        metadata.set_widget_name("external-editor-metadata");
        root.append(&labeled_row("Metadata", &metadata));

        let xmp = gtk4::CheckButton::with_label("Include XMP sidecar");
        xmp.set_widget_name("external-editor-xmp");
        let add_to_catalog = gtk4::CheckButton::with_label("Add derived photo to catalog");
        add_to_catalog.set_widget_name("external-editor-add-to-catalog");
        add_to_catalog.set_active(true);
        let group_with_source = gtk4::CheckButton::with_label("Group with source");
        group_with_source.set_widget_name("external-editor-group-with-source");
        group_with_source.set_active(true);
        root.append(&xmp);
        root.append(&add_to_catalog);
        root.append(&group_with_source);

        let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
        let save = gtk4::Button::with_label("Save preset");
        save.set_widget_name("external-editor-save");
        let test = gtk4::Button::with_label("Test preset");
        test.set_widget_name("external-editor-test");
        let send = gtk4::Button::with_label("Review send…");
        send.set_widget_name("external-editor-send");
        actions.append(&save);
        actions.append(&test);
        actions.append(&send);
        root.append(&actions);

        let selection = gtk4::Label::new(Some("No photos selected"));
        selection.set_halign(gtk4::Align::Start);
        selection.add_css_class("dim-label");
        root.append(&selection);
        let review = gtk4::Label::new(None);
        review.set_halign(gtk4::Align::Start);
        review.set_wrap(true);
        review.set_visible(false);
        review.set_widget_name("external-editor-review");
        root.append(&review);
        let confirm = gtk4::Button::with_label("Confirm send");
        confirm.set_widget_name("external-editor-confirm");
        confirm.set_visible(false);
        root.append(&confirm);

        let progress = gtk4::ProgressBar::new();
        progress.set_widget_name("external-editor-progress");
        progress.set_show_text(true);
        progress.set_accessible_role(gtk4::AccessibleRole::ProgressBar);
        root.append(&progress);
        let status = gtk4::Label::new(Some("No external-editor activity"));
        status.set_widget_name("external-editor-status");
        status.set_halign(gtk4::Align::Start);
        status.set_wrap(true);
        status.set_accessible_role(gtk4::AccessibleRole::Status);
        status.update_property(&[Property::Label("External editor status")]);
        root.append(&status);
        let job_actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
        let cancel = gtk4::Button::with_label("Cancel");
        cancel.set_widget_name("external-editor-cancel");
        let reconcile = gtk4::Button::with_label("Reconcile");
        reconcile.set_widget_name("external-editor-reconcile");
        job_actions.append(&cancel);
        job_actions.append(&reconcile);
        root.append(&job_actions);

        Self {
            root,
            preset_dropdown,
            name,
            executable,
            executable_status,
            interchange,
            arguments,
            add_literal,
            add_placeholder,
            profile,
            destination,
            bit_depth,
            metadata,
            xmp,
            add_to_catalog,
            group_with_source,
            save,
            test,
            send,
            confirm,
            cancel,
            reconcile,
            selection,
            review,
            progress,
            status,
            current_preset: Rc::new(RefCell::new(None)),
            preset_ids: Rc::new(RefCell::new(Vec::new())),
            draft_arguments: Rc::new(RefCell::new(Vec::new())),
            current_review: Rc::new(RefCell::new(None)),
            current_job: Rc::new(RefCell::new(None)),
        }
    }

    #[must_use]
    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    pub fn set_state(&self, state: &ExternalEditorViewModel) {
        let labels = state
            .presets()
            .iter()
            .map(|preset| preset.name().as_str())
            .collect::<Vec<_>>();
        let labels = if labels.is_empty() {
            vec!["No presets"]
        } else {
            labels
        };
        self.preset_ids.replace(
            state
                .presets()
                .iter()
                .map(ExternalEditorPreset::id)
                .collect(),
        );
        self.preset_dropdown
            .set_model(Some(&gtk4::StringList::new(&labels)));
        let selected = state
            .selected_preset()
            .and_then(|id| state.presets().iter().position(|preset| preset.id() == id))
            .unwrap_or(0);
        self.preset_dropdown
            .set_selected(u32::try_from(selected).unwrap_or_default());
        let preset = state
            .selected_preset()
            .and_then(|id| state.presets().iter().find(|preset| preset.id() == id));
        self.current_preset.replace(preset.cloned());
        if let Some(preset) = preset {
            self.draft_arguments.replace(preset.arguments().to_vec());
            self.project_preset(preset);
        } else {
            self.name.set_text("");
            self.executable_status.set_text("No executable approved");
            clear_children(&self.arguments);
            self.draft_arguments.borrow_mut().clear();
        }
        self.selection.set_text(&format!(
            "{} photo(s) selected",
            state.selected_photos().len()
        ));
        self.current_review.replace(state.review().cloned());
        if let Some(review) = state.review() {
            self.review.set_text(&format_review(review));
            self.review.set_visible(true);
            self.confirm.set_visible(true);
        } else {
            self.review.set_visible(false);
            self.confirm.set_visible(false);
        }
        let job = state.jobs().last().cloned();
        self.current_job.replace(job.clone());
        self.project_job(job.as_ref());
        self.status.set_text(state.announcement().as_str());
    }

    pub fn set_selection(&self, count: usize) {
        self.selection
            .set_text(&format!("{count} photo(s) selected"));
    }

    pub fn connect_action<F>(&self, handler: F)
    where
        F: Fn(ExternalEditorAction) + 'static,
    {
        let handler: Rc<ActionHandler> = Rc::new(Box::new(handler));
        let ids = Rc::clone(&self.preset_ids);
        let callback = Rc::clone(&handler);
        self.preset_dropdown
            .connect_selected_notify(move |dropdown| {
                let selected = usize::try_from(dropdown.selected()).ok();
                let preset = selected.and_then(|index| ids.borrow().get(index).copied());
                callback(ExternalEditorAction::SelectPreset(preset));
            });
        connect_button(&self.executable, Rc::clone(&handler), || {
            ExternalEditorAction::ChooseExecutable
        });
        connect_button(&self.save, Rc::clone(&handler), {
            let panel = self.clone();
            move || panel.draft_action()
        });
        connect_button(&self.test, Rc::clone(&handler), {
            let panel = self.clone();
            move || panel.selected_action(ExternalEditorAction::TestPreset)
        });
        connect_button(&self.send, Rc::clone(&handler), || {
            ExternalEditorAction::ReviewSend
        });
        connect_button(&self.confirm, Rc::clone(&handler), {
            let panel = self.clone();
            move || panel.confirm_action()
        });
        connect_button(&self.cancel, Rc::clone(&handler), {
            let panel = self.clone();
            move || panel.job_action(ExternalEditorAction::CancelJob)
        });
        connect_button(&self.reconcile, Rc::clone(&handler), {
            let panel = self.clone();
            move || panel.job_action(ExternalEditorAction::ReconcileJob)
        });
        self.add_literal.connect_clicked({
            let callback = Rc::clone(&handler);
            let panel = self.clone();
            move |_| {
                if let Ok(row) = ArgumentRow::literal("--edit") {
                    panel.add_argument(row);
                }
                callback(ExternalEditorAction::AddLiteralArgument);
            }
        });
        let callback = Rc::clone(&handler);
        self.add_placeholder.connect_clicked({
            let panel = self.clone();
            move |_| {
                panel.add_argument(ArgumentRow::placeholder(Placeholder::Input));
                callback(ExternalEditorAction::AddPlaceholderArgument(
                    Placeholder::Input,
                ));
            }
        });
    }

    fn draft_action(&self) -> ExternalEditorAction {
        let current_preset = self.current_preset.borrow();
        let Some(preset) = current_preset.as_ref() else {
            return ExternalEditorAction::NewPreset;
        };
        let mut draft = preset.draft();
        let Ok(name) = PresentationText::new(self.name.text().as_str()) else {
            return ExternalEditorAction::NewPreset;
        };
        let Ok(profile) = PresentationText::new(self.profile.text().as_str()) else {
            return ExternalEditorAction::NewPreset;
        };
        let Ok(destination) = PresentationText::new(self.destination.text().as_str()) else {
            return ExternalEditorAction::NewPreset;
        };
        draft.name = name;
        draft.profile = profile;
        draft.destination = destination;
        draft.arguments.clone_from(&self.draft_arguments.borrow());
        draft.interchange = if self.interchange.selected() == 0 {
            InterchangeMode::InPlaceTiff
        } else {
            InterchangeMode::SeparateOutputTiff
        };
        draft.bit_depth = if self.bit_depth.selected() == 0 {
            TiffBitDepth::Sixteen
        } else {
            TiffBitDepth::Float32
        };
        draft.metadata = if self.metadata.selected() == 0 {
            MetadataPolicy::Preserve
        } else {
            MetadataPolicy::Minimal
        };
        draft.include_xmp = self.xmp.is_active();
        draft.add_to_catalog = self.add_to_catalog.is_active();
        draft.group_with_source = self.group_with_source.is_active();
        ExternalEditorAction::SaveDraft(draft)
    }

    fn selected_action<F>(&self, build: F) -> ExternalEditorAction
    where
        F: FnOnce(PresetId) -> ExternalEditorAction,
    {
        self.current_preset
            .borrow()
            .as_ref()
            .map_or(ExternalEditorAction::NewPreset, |preset| build(preset.id()))
    }

    fn confirm_action(&self) -> ExternalEditorAction {
        self.current_review
            .borrow()
            .as_ref()
            .map_or(ExternalEditorAction::NewPreset, |review| {
                ExternalEditorAction::ConfirmSend(SendToEditorRequest {
                    preset: review.preset(),
                    photos: review.photos().to_vec(),
                    source_revision: review.source_revision(),
                })
            })
    }

    fn job_action<F>(&self, build: F) -> ExternalEditorAction
    where
        F: FnOnce(super::model::JobId) -> ExternalEditorAction,
    {
        self.current_job
            .borrow()
            .as_ref()
            .map_or(ExternalEditorAction::NewPreset, |job| build(job.id()))
    }

    fn project_preset(&self, preset: &ExternalEditorPreset) {
        self.name.set_text(preset.name().as_str());
        self.executable_status
            .set_text(preset.executable().approval().label());
        self.interchange.set_selected(u32::from(
            preset.interchange() != InterchangeMode::InPlaceTiff,
        ));
        self.profile.set_text(preset.profile().as_str());
        self.destination.set_text(preset.destination().as_str());
        self.bit_depth
            .set_selected(u32::from(preset.bit_depth() != TiffBitDepth::Sixteen));
        self.metadata
            .set_selected(u32::from(preset.metadata() != MetadataPolicy::Preserve));
        self.xmp.set_active(preset.include_xmp());
        self.add_to_catalog.set_active(preset.add_to_catalog());
        self.group_with_source
            .set_active(preset.group_with_source());
        clear_children(&self.arguments);
        for row in self.draft_arguments.borrow().iter() {
            let label = gtk4::Label::new(Some(row.display_token()));
            label.set_halign(gtk4::Align::Start);
            self.arguments.append(&label);
        }
    }

    fn add_argument(&self, row: ArgumentRow) {
        self.draft_arguments.borrow_mut().push(row);
        let rows = self.draft_arguments.borrow().clone();
        clear_children(&self.arguments);
        for row in rows {
            let label = gtk4::Label::new(Some(row.display_token()));
            label.set_halign(gtk4::Align::Start);
            self.arguments.append(&label);
        }
    }

    fn project_job(&self, job: Option<&ExternalEditorJob>) {
        if let Some(job) = job {
            self.progress
                .set_fraction(f64::from(job.progress_percent()) / 100.0);
            self.progress
                .set_text(Some(&format!("{}%", job.progress_percent())));
            self.status.set_text(&format!(
                "{}: {}",
                job.stage().label(),
                job.detail().as_str()
            ));
        } else {
            self.progress.set_fraction(0.0);
            self.progress.set_text(Some("0%"));
        }
    }
}

impl Default for ExternalEditorPanel {
    fn default() -> Self {
        Self::new()
    }
}

fn labeled_row(label: &str, widget: &impl IsA<gtk4::Widget>) -> gtk4::Box {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    let label = gtk4::Label::new(Some(label));
    label.set_width_chars(14);
    label.set_halign(gtk4::Align::Start);
    label.set_hexpand(false);
    row.append(&label);
    row.append(widget);
    row
}

fn indent(widget: &impl IsA<gtk4::Widget>) -> gtk4::Box {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    row.set_margin_start(90);
    row.append(widget);
    row
}

fn connect_button<F>(button: &gtk4::Button, handler: Rc<ActionHandler>, action: F)
where
    F: Fn() -> ExternalEditorAction + 'static,
{
    button.connect_clicked(move |_| handler(action()));
}

fn format_review(review: &InvocationReview) -> String {
    format!(
        "Review: {} photo(s) · {} · {} · {} · destination {}",
        review.photos().len(),
        review.interchange().label(),
        review.bit_depth().label(),
        review.profile().as_str(),
        review.destination().as_str()
    )
}

fn clear_children(container: &impl IsA<gtk4::Widget>) {
    while let Some(child) = container.first_child() {
        child.unparent();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external_editor::JobStage;

    #[test]
    fn focus_order_names_real_controls_and_excludes_private_paths() {
        assert_eq!(EXTERNAL_EDITOR_FOCUS_ORDER[0], "external-editor-preset");
        assert!(EXTERNAL_EDITOR_FOCUS_ORDER.contains(&"external-editor-send"));
        assert!(
            !EXTERNAL_EDITOR_FOCUS_ORDER
                .iter()
                .any(|value| value.contains("path"))
        );
    }

    #[test]
    fn job_labels_explain_owned_child_completion() {
        assert!(
            JobStage::ExternalAppRunning
                .label()
                .contains("save and close")
        );
    }
}
