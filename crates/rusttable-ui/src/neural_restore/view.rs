//! GTK4 comparison view. Only validated presentation frames cross into the widgets.

#![allow(clippy::missing_panics_doc)]
#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::accessible::Property;
use gtk4::prelude::*;

use crate::ai_models::{AiProvider, ModelHash};

use super::model::{
    ComparisonMode, NeuralRestoreAction, NeuralRestoreSnapshot, NeuralRestoreViewModel,
    PreviewEligibility, PreviewFrame, PreviewStage, PreviewStatus, RestoreTask,
};

type ActionHandler = Rc<dyn Fn(NeuralRestoreAction)>;

#[derive(Clone)]
pub struct NeuralRestorePanel {
    root: gtk4::Box,
    task: gtk4::DropDown,
    model: gtk4::DropDown,
    provider: gtk4::DropDown,
    strength: gtk4::Scale,
    wide_gamut: gtk4::CheckButton,
    scale: gtk4::DropDown,
    comparison: gtk4::DropDown,
    split: gtk4::Scale,
    source: gtk4::Picture,
    restored: gtk4::Picture,
    source_label: gtk4::Label,
    restored_label: gtk4::Label,
    cancel: gtk4::Button,
    progress: gtk4::ProgressBar,
    status: gtk4::Label,
    models: Rc<RefCell<Vec<ModelHash>>>,
}

impl NeuralRestorePanel {
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn new() -> Self {
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        root.set_widget_name("neural-restore");
        root.set_margin_top(6);
        root.set_margin_bottom(6);
        root.set_margin_start(6);
        root.set_margin_end(6);
        root.set_accessible_role(gtk4::AccessibleRole::Region);
        root.update_property(&[Property::Label("Neural restore preview")]);

        let heading = gtk4::Label::new(Some("Neural restore"));
        heading.set_halign(gtk4::Align::Start);
        heading.add_css_class("title-4");
        root.append(&heading);
        let hint = gtk4::Label::new(Some(
            "One selected photo · preview only · no files or catalog changes",
        ));
        hint.set_halign(gtk4::Align::Start);
        hint.add_css_class("dim-label");
        root.append(&hint);

        let task = gtk4::DropDown::from_strings(&RestoreTask::all().map(RestoreTask::label));
        task.set_widget_name("neural-restore-task");
        root.append(&labeled_row("Task", &task));
        let model = gtk4::DropDown::from_strings(&["No enabled model"]);
        model.set_widget_name("neural-restore-model");
        root.append(&labeled_row("Model", &model));
        let provider = gtk4::DropDown::from_strings(&["No qualified provider"]);
        provider.set_widget_name("neural-restore-provider");
        root.append(&labeled_row("Provider", &provider));

        let strength = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 0.0, 100.0, 1.0);
        strength.set_widget_name("neural-restore-strength");
        strength.set_value(50.0);
        strength.set_draw_value(true);
        strength.set_hexpand(true);
        strength.set_accessible_role(gtk4::AccessibleRole::Slider);
        root.append(&labeled_row("Strength", &strength));
        let wide_gamut = gtk4::CheckButton::with_label("Preserve wide gamut (RGB denoise)");
        wide_gamut.set_widget_name("neural-restore-wide-gamut");
        wide_gamut.set_active(true);
        root.append(&wide_gamut);
        let scale = gtk4::DropDown::from_strings(&["2×", "4×"]);
        scale.set_widget_name("neural-restore-scale");
        root.append(&labeled_row("Upscale", &scale));

        let comparison =
            gtk4::DropDown::from_strings(&ComparisonMode::all().map(ComparisonMode::label));
        comparison.set_widget_name("neural-restore-comparison");
        root.append(&labeled_row("Compare", &comparison));
        let split = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 0.0, 100.0, 1.0);
        split.set_widget_name("neural-restore-split");
        split.set_value(50.0);
        split.set_draw_value(true);
        split.set_sensitive(false);
        split.set_tooltip_text(Some(
            "Split position; use arrow keys for precise adjustment",
        ));
        root.append(&labeled_row("Split", &split));

        let panes = gtk4::Paned::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .position(360)
            .build();
        let (source, source_picture, source_label) =
            pane("neural-restore-source", "Source / current edit");
        let (restored, restored_picture, restored_label) =
            pane("neural-restore-restored", "Restored preview");
        panes.set_start_child(Some(&source));
        panes.set_end_child(Some(&restored));
        panes.set_vexpand(true);
        root.append(&panes);

        let progress = gtk4::ProgressBar::new();
        progress.set_widget_name("neural-restore-progress");
        progress.set_show_text(true);
        progress.set_accessible_role(gtk4::AccessibleRole::ProgressBar);
        root.append(&progress);
        let cancel = gtk4::Button::with_label("Cancel preview");
        cancel.set_widget_name("neural-restore-cancel");
        root.append(&cancel);
        let status = gtk4::Label::new(Some(PreviewEligibility::ServiceUnavailable.label()));
        status.set_widget_name("neural-restore-status");
        status.set_halign(gtk4::Align::Start);
        status.set_wrap(true);
        status.set_accessible_role(gtk4::AccessibleRole::Status);
        status.update_property(&[Property::Label("Neural restore preview status")]);
        root.append(&status);

        Self {
            root,
            task,
            model,
            provider,
            strength,
            wide_gamut,
            scale,
            comparison,
            split,
            source: source_picture,
            restored: restored_picture,
            source_label,
            restored_label,
            cancel,
            progress,
            status,
            models: Rc::new(RefCell::new(Vec::new())),
        }
    }

    #[must_use]
    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    pub fn set_state(&self, state: &NeuralRestoreViewModel) {
        self.status.set_text(state.announcement());
        self.task.set_selected(task_index(state.task()));
        self.strength
            .set_value(if state.task() == RestoreTask::RawDenoise {
                f64::from(state.settings().raw_strength())
            } else {
                f64::from(state.settings().rgb_strength())
            });
        self.wide_gamut
            .set_active(state.settings().preserve_wide_gamut());
        self.scale
            .set_selected(u32::from(state.settings().scale() == 4));
        self.comparison
            .set_selected(comparison_index(state.comparison()));
        self.split
            .set_value(f64::from(state.viewport().split()) * 100.0);
        self.split
            .set_sensitive(state.comparison() == ComparisonMode::Split);
        self.render_models(state.snapshot(), state.task());
        if let Some(artifact) = state.artifact() {
            Self::set_frame(&self.source, artifact.source());
            Self::set_frame(&self.restored, artifact.restored());
            self.source_label.set_text("Source / current edit");
            self.restored_label.set_text(if artifact.cache_hit() {
                "Restored preview · cache hit"
            } else {
                "Restored preview"
            });
        } else {
            Self::clear_frame(&self.source, &self.source_label, "Source / current edit");
            Self::clear_frame(
                &self.restored,
                &self.restored_label,
                "Preview unavailable until the service is connected",
            );
        }
        match state.status() {
            PreviewStatus::Running(preview_stage) => {
                self.progress.pulse();
                self.progress.set_text(Some(preview_stage.label()));
                self.cancel.set_sensitive(true);
            }
            PreviewStatus::Debouncing => {
                self.progress.set_fraction(0.0);
                self.progress
                    .set_text(Some(PreviewStage::Debouncing.label()));
                self.cancel.set_sensitive(true);
            }
            PreviewStatus::Ready | PreviewStatus::CacheHit => {
                self.progress.set_fraction(1.0);
                self.progress.set_text(Some("Preview ready"));
                self.cancel.set_sensitive(false);
            }
            _ => {
                self.progress.set_fraction(0.0);
                self.progress.set_text(Some("No preview job"));
                self.cancel.set_sensitive(false);
            }
        }
    }

    fn render_models(&self, snapshot: &NeuralRestoreSnapshot, task: RestoreTask) {
        let values = snapshot.models(task).collect::<Vec<_>>();
        let labels = if values.is_empty() {
            vec!["No enabled model".to_owned()]
        } else {
            values
                .iter()
                .map(|(_, label)| (*label).to_owned())
                .collect()
        };
        self.models
            .replace(values.iter().map(|(hash, _)| (*hash).clone()).collect());
        self.model.set_model(Some(&gtk4::StringList::new(
            &labels.iter().map(String::as_str).collect::<Vec<_>>(),
        )));
        self.model.set_selected(0);
        let providers = if snapshot.providers().is_empty() {
            vec!["No qualified provider".to_owned()]
        } else {
            snapshot
                .providers()
                .iter()
                .map(|provider| provider.label().to_owned())
                .collect()
        };
        self.provider.set_model(Some(&gtk4::StringList::new(
            &providers.iter().map(String::as_str).collect::<Vec<_>>(),
        )));
        self.provider.set_selected(0);
    }

    fn set_frame(picture: &gtk4::Picture, frame: &PreviewFrame) {
        let Ok(width) = i32::try_from(frame.width()) else {
            return;
        };
        let Ok(height) = i32::try_from(frame.height()) else {
            return;
        };
        let Some(stride) = usize::try_from(frame.width())
            .ok()
            .and_then(|value| value.checked_mul(4))
        else {
            return;
        };
        let bytes = gtk4::glib::Bytes::from_owned(frame.pixels().to_owned());
        let texture = gtk4::gdk::MemoryTexture::new(
            width,
            height,
            gtk4::gdk::MemoryFormat::R8g8b8a8,
            &bytes,
            stride,
        );
        picture.set_paintable(Some(&texture));
    }

    fn clear_frame(picture: &gtk4::Picture, label: &gtk4::Label, text: &str) {
        picture.set_paintable(None::<&gtk4::gdk::Texture>);
        label.set_text(text);
    }

    pub fn connect_action<F>(&self, handler: F)
    where
        F: Fn(NeuralRestoreAction) + 'static,
    {
        let handler: ActionHandler = Rc::new(handler);
        let callback = Rc::clone(&handler);
        self.task.connect_selected_notify(move |dropdown| {
            if let Some(task) = RestoreTask::all().get(dropdown.selected() as usize) {
                callback(NeuralRestoreAction::SelectTask(*task));
            }
        });
        let callback = Rc::clone(&handler);
        let models = Rc::clone(&self.models);
        self.model.connect_selected_notify(move |dropdown| {
            callback(NeuralRestoreAction::SelectModel(
                models.borrow().get(dropdown.selected() as usize).cloned(),
            ));
        });
        let callback = Rc::clone(&handler);
        self.provider.connect_selected_notify(move |dropdown| {
            if let Some(provider) = [
                AiProvider::Cpu,
                AiProvider::CoreMl,
                AiProvider::DirectMl,
                AiProvider::Cuda,
            ]
            .get(dropdown.selected() as usize)
            {
                callback(NeuralRestoreAction::SelectProvider(*provider));
            }
        });
        let callback = Rc::clone(&handler);
        self.strength.connect_value_changed(move |scale| {
            callback(NeuralRestoreAction::SetRgbStrength(
                scale.value().round().clamp(0.0, 100.0) as u8,
            ));
        });
        let callback = Rc::clone(&handler);
        self.wide_gamut.connect_toggled(move |check| {
            callback(NeuralRestoreAction::SetWideGamut(check.is_active()));
        });
        let callback = Rc::clone(&handler);
        self.scale.connect_selected_notify(move |dropdown| {
            callback(NeuralRestoreAction::SetScale(if dropdown.selected() == 1 {
                4
            } else {
                2
            }));
        });
        let callback = Rc::clone(&handler);
        self.comparison.connect_selected_notify(move |dropdown| {
            if let Some(mode) = ComparisonMode::all().get(dropdown.selected() as usize) {
                callback(NeuralRestoreAction::SetComparison(*mode));
            }
        });
        let callback = Rc::clone(&handler);
        self.split.connect_value_changed(move |scale| {
            callback(NeuralRestoreAction::AdjustSplit(
                (scale.value() as i8).saturating_sub(50),
            ));
        });
        let callback = Rc::clone(&handler);
        self.cancel
            .connect_clicked(move |_| callback(NeuralRestoreAction::Cancel));
    }
}

impl Default for NeuralRestorePanel {
    fn default() -> Self {
        Self::new()
    }
}

fn pane(id: &str, title: &str) -> (gtk4::Box, gtk4::Picture, gtk4::Label) {
    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    root.set_widget_name(id);
    root.set_hexpand(true);
    root.set_vexpand(true);
    let label = gtk4::Label::new(Some(title));
    label.set_halign(gtk4::Align::Start);
    label.add_css_class("dim-label");
    let picture = gtk4::Picture::new();
    picture.set_widget_name(&format!("{id}-preview"));
    picture.set_content_fit(gtk4::ContentFit::Contain);
    picture.set_hexpand(true);
    picture.set_vexpand(true);
    root.append(&label);
    root.append(&picture);
    (root, picture, label)
}

fn labeled_row(label: &str, widget: &impl IsA<gtk4::Widget>) -> gtk4::Box {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    let label = gtk4::Label::new(Some(label));
    label.set_width_chars(12);
    label.set_halign(gtk4::Align::Start);
    row.append(&label);
    row.append(widget);
    row
}
fn task_index(task: RestoreTask) -> u32 {
    RestoreTask::all()
        .iter()
        .position(|item| *item == task)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or_default()
}
fn comparison_index(mode: ComparisonMode) -> u32 {
    ComparisonMode::all()
        .iter()
        .position(|item| *item == mode)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or_default()
}
