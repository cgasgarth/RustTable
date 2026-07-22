//! GTK4 projection for the linear RAW denoise controller state.

#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::too_many_lines)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::accessible::Property;
use gtk4::prelude::*;

use super::model::{
    RAW_DENOISE_MAX_STRENGTH, RAW_DENOISE_TILES, RawDenoiseAction, RawDenoiseMemoryState,
    RawDenoiseOutputPolicy, RawDenoisePlanPolicy, RawDenoiseStatus, RawDenoiseViewModel,
};
use crate::ai_models::{AiProvider, ModelHash};

type ActionHandler = Rc<dyn Fn(RawDenoiseAction)>;

#[derive(Clone)]
pub struct RawDenoisePanel {
    root: gtk4::Box,
    model: gtk4::DropDown,
    provider: gtk4::DropDown,
    strength: gtk4::Scale,
    tile: gtk4::DropDown,
    plan_policy: gtk4::DropDown,
    output_policy: gtk4::DropDown,
    preview: gtk4::Button,
    full: gtk4::Button,
    export: gtk4::Button,
    cancel: gtk4::Button,
    progress: gtk4::ProgressBar,
    plan: gtk4::Label,
    memory: gtk4::Label,
    source: gtk4::Label,
    calibration: gtk4::Label,
    profile: gtk4::Label,
    status: gtk4::Label,
    model_hashes: Rc<RefCell<Vec<ModelHash>>>,
    signal_guard: Rc<Cell<bool>>,
}

impl RawDenoisePanel {
    #[must_use]
    pub fn new() -> Self {
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 5);
        root.set_widget_name("raw-denoise");
        root.set_margin_top(6);
        root.set_margin_bottom(6);
        root.set_margin_start(6);
        root.set_margin_end(6);
        root.set_accessible_role(gtk4::AccessibleRole::Region);
        root.update_property(&[Property::Label("RAW AI denoise")]);

        let heading = gtk4::Label::new(Some("RAW AI denoise (linear)"));
        heading.set_halign(gtk4::Align::Start);
        heading.add_css_class("title-4");
        root.append(&heading);
        let hint = gtk4::Label::new(Some(
            "X-Trans and already-linear sources · app service owns inference, DNG publication, and import",
        ));
        hint.set_halign(gtk4::Align::Start);
        hint.set_wrap(true);
        hint.add_css_class("dim-label");
        root.append(&hint);

        let model = gtk4::DropDown::from_strings(&["No qualified RawLinearDenoise model"]);
        model.set_widget_name("raw-denoise-model");
        root.append(&row("Qualified model", &model));
        let provider = gtk4::DropDown::from_strings(&["No qualified provider"]);
        provider.set_widget_name("raw-denoise-provider");
        root.append(&row("Provider", &provider));

        let source = gtk4::Label::new(Some("Source: unavailable"));
        source.set_widget_name("raw-denoise-source");
        source.set_halign(gtk4::Align::Start);
        source.set_wrap(true);
        root.append(&source);
        let calibration = gtk4::Label::new(Some("Calibration: unavailable"));
        calibration.set_widget_name("raw-denoise-calibration");
        calibration.set_halign(gtk4::Align::Start);
        root.append(&calibration);
        let profile = gtk4::Label::new(Some("Profile: unavailable"));
        profile.set_widget_name("raw-denoise-profile");
        profile.set_halign(gtk4::Align::Start);
        root.append(&profile);

        let strength = gtk4::Scale::with_range(
            gtk4::Orientation::Horizontal,
            0.0,
            f64::from(RAW_DENOISE_MAX_STRENGTH),
            1.0,
        );
        strength.set_widget_name("raw-denoise-strength");
        strength.set_value(50.0);
        strength.set_draw_value(true);
        strength.set_hexpand(true);
        root.append(&row("Strength", &strength));
        let tile = gtk4::DropDown::from_strings(&["128 px", "256 px", "512 px"]);
        tile.set_widget_name("raw-denoise-tile");
        root.append(&row("Tile", &tile));
        let plan_policy = gtk4::DropDown::from_strings(&["Minimal RAW plan"]);
        plan_policy.set_widget_name("raw-denoise-plan-policy");
        root.append(&row("Plan", &plan_policy));
        let output_policy = gtk4::DropDown::from_strings(&[
            "Preview buffer only",
            "Publish DNG",
            "Publish and import DNG",
        ]);
        output_policy.set_widget_name("raw-denoise-output-policy");
        root.append(&row("Output", &output_policy));

        let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 5);
        let preview = gtk4::Button::with_label("Preview");
        preview.set_widget_name("raw-denoise-preview");
        let full = gtk4::Button::with_label("Run full");
        full.set_widget_name("raw-denoise-full");
        let export = gtk4::Button::with_label("Export");
        export.set_widget_name("raw-denoise-export");
        let cancel = gtk4::Button::with_label("Cancel");
        cancel.set_widget_name("raw-denoise-cancel");
        for button in [&preview, &full, &export, &cancel] {
            actions.append(button);
        }
        root.append(&actions);

        let progress = gtk4::ProgressBar::new();
        progress.set_widget_name("raw-denoise-progress");
        progress.set_show_text(true);
        progress.set_accessible_role(gtk4::AccessibleRole::ProgressBar);
        root.append(&progress);
        let plan = gtk4::Label::new(Some("Plan: unavailable"));
        plan.set_widget_name("raw-denoise-plan");
        plan.set_halign(gtk4::Align::Start);
        plan.set_wrap(true);
        root.append(&plan);
        let memory = gtk4::Label::new(Some("Memory: unavailable"));
        memory.set_widget_name("raw-denoise-memory");
        memory.set_halign(gtk4::Align::Start);
        memory.set_wrap(true);
        root.append(&memory);
        let status = gtk4::Label::new(Some(
            "RAW linear denoise app service unavailable; no inference or DNG/catalog write was performed.",
        ));
        status.set_widget_name("raw-denoise-status");
        status.set_halign(gtk4::Align::Start);
        status.set_wrap(true);
        status.set_accessible_role(gtk4::AccessibleRole::Status);
        root.append(&status);

        Self {
            root,
            model,
            provider,
            strength,
            tile,
            plan_policy,
            output_policy,
            preview,
            full,
            export,
            cancel,
            progress,
            plan,
            memory,
            source,
            calibration,
            profile,
            status,
            model_hashes: Rc::new(RefCell::new(Vec::new())),
            signal_guard: Rc::new(Cell::new(false)),
        }
    }

    #[must_use]
    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    pub fn set_state(&self, state: &RawDenoiseViewModel) {
        self.signal_guard.set(true);
        self.set_models(state);
        self.set_providers(state);
        self.strength.set_value(f64::from(state.strength()));
        self.tile.set_selected(tile_index(state.tile_size()));
        self.plan_policy.set_selected(0);
        self.output_policy
            .set_selected(output_index(state.output_policy()));
        self.signal_guard.set(false);

        let source_info = state.snapshot().source();
        self.source.set_text(&format!(
            "Source: {} · {} · {:?}",
            source_info.layout().label(),
            source_info.source_identity(),
            source_info.dimensions().unwrap_or_default(),
        ));
        self.calibration.set_text(&format!(
            "Calibration: {}",
            source_info.calibration().label()
        ));
        self.profile
            .set_text(&format!("Profile: {}", source_info.profile().label()));
        if let Some(plan) = state.plan() {
            self.plan.set_text(&format!(
                "Plan: {} · {} · {} tile · {}",
                plan.identity(),
                plan.layout().label(),
                plan.tile_size(),
                plan.output_policy().label(),
            ));
            self.memory.set_text(&format!(
                "Memory estimate: {}",
                format_bytes(plan.memory_bytes())
            ));
        } else {
            self.plan.set_text("Plan: unavailable");
            self.memory.set_text(&memory_text(state.memory_state()));
        }
        self.status.set_text(&status_text(state));
        if let Some(progress) = state.progress() {
            self.progress.set_fraction(progress.fraction());
            self.progress.set_text(Some(&format!(
                "{} / {}",
                progress.completed, progress.total
            )));
        } else {
            self.progress.set_fraction(0.0);
            self.progress.set_text(Some("No active job"));
        }
        let running = matches!(
            state.status(),
            RawDenoiseStatus::Running { .. }
                | RawDenoiseStatus::Cancelling
                | RawDenoiseStatus::PendingPublication { .. }
        );
        let ready = state.plan().is_some() && !running;
        self.preview.set_sensitive(ready);
        self.full.set_sensitive(ready);
        self.export.set_sensitive(ready);
        self.cancel.set_sensitive(running);
        self.strength.set_sensitive(!running);
        self.tile.set_sensitive(!running);
        self.output_policy.set_sensitive(!running);
    }

    pub fn connect_action<F>(&self, callback: F)
    where
        F: Fn(RawDenoiseAction) + 'static,
    {
        let callback: ActionHandler = Rc::new(callback);
        connect_dropdown(
            &self.model,
            Rc::clone(&callback),
            Rc::clone(&self.signal_guard),
            {
                let hashes = Rc::clone(&self.model_hashes);
                move |index| RawDenoiseAction::SelectModel(hashes.borrow().get(index).cloned())
            },
        );
        connect_dropdown(
            &self.provider,
            Rc::clone(&callback),
            Rc::clone(&self.signal_guard),
            move |index| RawDenoiseAction::SelectProvider(AiProvider::all().get(index).copied()),
        );
        let guard = Rc::clone(&self.signal_guard);
        let callback_for_strength = Rc::clone(&callback);
        self.strength.connect_value_changed(move |scale| {
            if !guard.get() {
                callback_for_strength(RawDenoiseAction::SetStrength(
                    scale
                        .value()
                        .round()
                        .clamp(0.0, f64::from(RAW_DENOISE_MAX_STRENGTH)) as u8,
                ));
            }
        });
        connect_dropdown(
            &self.tile,
            Rc::clone(&callback),
            Rc::clone(&self.signal_guard),
            move |index| RawDenoiseAction::SetTile(RAW_DENOISE_TILES[index]),
        );
        connect_dropdown(
            &self.plan_policy,
            Rc::clone(&callback),
            Rc::clone(&self.signal_guard),
            move |_| RawDenoiseAction::SetPlanPolicy(RawDenoisePlanPolicy::MinimalRaw),
        );
        connect_dropdown(
            &self.output_policy,
            Rc::clone(&callback),
            Rc::clone(&self.signal_guard),
            move |index| RawDenoiseAction::SetOutputPolicy(output_policy_at(index)),
        );
        connect_button(
            &self.preview,
            Rc::clone(&callback),
            RawDenoiseAction::Preview,
        );
        connect_button(&self.full, Rc::clone(&callback), RawDenoiseAction::Full);
        connect_button(&self.export, Rc::clone(&callback), RawDenoiseAction::Export);
        connect_button(&self.cancel, callback, RawDenoiseAction::Cancel);
    }

    fn set_models(&self, state: &RawDenoiseViewModel) {
        let options = state.snapshot().qualified_models().collect::<Vec<_>>();
        self.model_hashes
            .replace(options.iter().map(|option| option.hash().clone()).collect());
        let labels = options
            .iter()
            .map(|option| option.label())
            .collect::<Vec<_>>();
        self.model.set_model(Some(&gtk4::StringList::new(&labels)));
        self.model.set_selected(
            options
                .iter()
                .position(|option| Some(option.hash()) == state.model())
                .unwrap_or(0) as u32,
        );
    }

    fn set_providers(&self, state: &RawDenoiseViewModel) {
        let labels = state
            .snapshot()
            .providers()
            .iter()
            .map(|provider| provider.label())
            .collect::<Vec<_>>();
        self.provider
            .set_model(Some(&gtk4::StringList::new(&labels)));
        self.provider.set_selected(
            state
                .provider()
                .and_then(|provider| {
                    state
                        .snapshot()
                        .providers()
                        .iter()
                        .position(|entry| *entry == provider)
                })
                .unwrap_or(0) as u32,
        );
    }
}

impl Default for RawDenoisePanel {
    fn default() -> Self {
        Self::new()
    }
}

fn row(label: &str, widget: &impl IsA<gtk4::Widget>) -> gtk4::Box {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 5);
    row.set_width_request(0);
    row.set_hexpand(true);
    let label_widget = gtk4::Label::new(Some(label));
    label_widget.set_width_chars(1);
    label_widget.set_hexpand(true);
    label_widget.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    label_widget.set_halign(gtk4::Align::Start);
    widget.set_width_request(0);
    widget.set_hexpand(true);
    row.append(&label_widget);
    row.append(widget);
    row
}

fn connect_dropdown<F>(
    dropdown: &gtk4::DropDown,
    callback: ActionHandler,
    guard: Rc<Cell<bool>>,
    action: F,
) where
    F: Fn(usize) -> RawDenoiseAction + 'static,
{
    dropdown.connect_selected_notify(move |dropdown| {
        if !guard.get() {
            callback(action(dropdown.selected() as usize));
        }
    });
}

fn connect_button(button: &gtk4::Button, callback: ActionHandler, action: RawDenoiseAction) {
    button.connect_clicked(move |_| callback(action.clone()));
}

fn tile_index(tile: u32) -> u32 {
    RAW_DENOISE_TILES
        .iter()
        .position(|candidate| *candidate == tile)
        .unwrap_or(1) as u32
}

fn output_index(policy: RawDenoiseOutputPolicy) -> u32 {
    match policy {
        RawDenoiseOutputPolicy::PreviewBuffer => 0,
        RawDenoiseOutputPolicy::PublishDng => 1,
        RawDenoiseOutputPolicy::PublishAndImport => 2,
    }
}

fn output_policy_at(index: usize) -> RawDenoiseOutputPolicy {
    match index {
        1 => RawDenoiseOutputPolicy::PublishDng,
        2 => RawDenoiseOutputPolicy::PublishAndImport,
        _ => RawDenoiseOutputPolicy::PreviewBuffer,
    }
}

fn status_text(state: &RawDenoiseViewModel) -> String {
    match state.status() {
        RawDenoiseStatus::Failed(error) => error.message(),
        RawDenoiseStatus::Running { kind, .. } => {
            format!("{} running through the application service.", kind.label())
        }
        RawDenoiseStatus::Cancelling => {
            "Cancellation requested; waiting for the service; staged output will be discarded."
                .to_owned()
        }
        RawDenoiseStatus::Cancelled => {
            "RAW denoise job cancelled; no staged output was published.".to_owned()
        }
        RawDenoiseStatus::PendingPublication { kind, artifact } => format!(
            "{} complete; publication pending for {artifact}.",
            kind.label()
        ),
        RawDenoiseStatus::Completed { kind, artifact } => format!(
            "{} completed; DNG publication receipt: {}.",
            kind.label(),
            artifact.as_deref().unwrap_or("not requested")
        ),
        RawDenoiseStatus::Imported { kind, artifact } => format!(
            "{} completed and imported as {artifact}; catalog state came from the app service.",
            kind.label()
        ),
        RawDenoiseStatus::Planning => {
            "Planning bounded RAW resources and validating source contracts…".to_owned()
        }
        RawDenoiseStatus::Ready => "Ready; choose preview, full render, or export.".to_owned(),
        RawDenoiseStatus::Idle => {
            "Select a qualified RawLinearDenoise model and a supported RAW source.".to_owned()
        }
    }
}

fn memory_text(state: RawDenoiseMemoryState) -> String {
    match state {
        RawDenoiseMemoryState::Unknown => "Memory estimate: unavailable".to_owned(),
        RawDenoiseMemoryState::Estimated { bytes } => {
            format!("Memory estimate: {}", format_bytes(bytes))
        }
        RawDenoiseMemoryState::Exceeded { bytes, limit } => format!(
            "Memory blocked: {} exceeds {}",
            format_bytes(bytes),
            format_bytes(limit)
        ),
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GiB", bytes as f64 / 1024.0 / 1024.0 / 1024.0)
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / 1024.0 / 1024.0)
    } else {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    }
}

#[cfg(test)]
mod tests {
    use super::super::model::{RawDenoiseFailure, RawDenoiseSourceLayout};
    use super::*;

    #[test]
    fn raw_controls_and_status_copy_are_stable() {
        assert_eq!(RAW_DENOISE_TILES, [128, 256, 512]);
        assert_eq!(format_bytes(1024 * 1024), "1.0 MiB");
        assert!(
            RawDenoiseFailure::UnsupportedLayout(RawDenoiseSourceLayout::XTrans)
                .message()
                .contains("not supported")
        );
        assert_eq!(
            output_policy_at(2),
            RawDenoiseOutputPolicy::PublishAndImport
        );
    }
}
