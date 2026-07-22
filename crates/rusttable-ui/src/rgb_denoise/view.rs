//! GTK4 projection for the RGB denoise controller state.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::too_many_lines
)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::accessible::Property;
use gtk4::prelude::*;

use super::model::{
    RGB_DENOISE_MAX_STRENGTH, RGB_DENOISE_SCALES, RGB_DENOISE_TILES, RgbDenoiseAction,
    RgbDenoiseDetailPolicy, RgbDenoiseGamutPolicy, RgbDenoiseShadowPolicy, RgbDenoiseStatus,
    RgbDenoiseViewModel,
};
use crate::ai_models::{AiProvider, ModelHash};

type ActionHandler = Rc<dyn Fn(RgbDenoiseAction)>;

#[derive(Clone)]
pub struct RgbDenoisePanel {
    root: gtk4::Box,
    model: gtk4::DropDown,
    provider: gtk4::DropDown,
    working_profile: gtk4::DropDown,
    model_profile: gtk4::DropDown,
    scale: gtk4::DropDown,
    tile: gtk4::DropDown,
    strength: gtk4::Scale,
    gamut: gtk4::DropDown,
    shadows: gtk4::DropDown,
    detail: gtk4::DropDown,
    detail_strength: gtk4::Scale,
    preview: gtk4::Button,
    full: gtk4::Button,
    export: gtk4::Button,
    cancel: gtk4::Button,
    progress: gtk4::ProgressBar,
    plan: gtk4::Label,
    memory: gtk4::Label,
    status: gtk4::Label,
    model_hashes: Rc<RefCell<Vec<ModelHash>>>,
    working_ids: Rc<RefCell<Vec<String>>>,
    model_ids: Rc<RefCell<Vec<String>>>,
    signal_guard: Rc<Cell<bool>>,
}

impl RgbDenoisePanel {
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn new() -> Self {
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 5);
        root.set_widget_name("rgb-denoise");
        root.set_margin_top(6);
        root.set_margin_bottom(6);
        root.set_margin_start(6);
        root.set_margin_end(6);
        root.set_accessible_role(gtk4::AccessibleRole::Region);
        root.update_property(&[Property::Label("RGB AI denoise")]);

        let heading = gtk4::Label::new(Some("RGB AI denoise"));
        heading.set_halign(gtk4::Align::Start);
        heading.add_css_class("title-4");
        root.append(&heading);
        let hint = gtk4::Label::new(Some(
            "Darkroom render boundary · source edit and catalog stay unchanged until export",
        ));
        hint.set_halign(gtk4::Align::Start);
        hint.set_wrap(true);
        hint.add_css_class("dim-label");
        root.append(&hint);

        let model = gtk4::DropDown::from_strings(&["No qualified RGB model"]);
        model.set_widget_name("rgb-denoise-model");
        root.append(&row("Qualified model", &model));
        let provider = gtk4::DropDown::from_strings(&["No qualified provider"]);
        provider.set_widget_name("rgb-denoise-provider");
        root.append(&row("Provider", &provider));
        let working_profile = gtk4::DropDown::from_strings(&["No working profile"]);
        working_profile.set_widget_name("rgb-denoise-working-profile");
        root.append(&row("Working profile", &working_profile));
        let model_profile = gtk4::DropDown::from_strings(&["No model profile"]);
        model_profile.set_widget_name("rgb-denoise-model-profile");
        root.append(&row("Model profile", &model_profile));

        let scale = gtk4::DropDown::from_strings(&["1×"]);
        scale.set_widget_name("rgb-denoise-scale");
        root.append(&row("Scale", &scale));
        let tile = gtk4::DropDown::from_strings(&["128 px", "256 px", "512 px"]);
        tile.set_widget_name("rgb-denoise-tile");
        root.append(&row("Tile", &tile));
        let strength = gtk4::Scale::with_range(
            gtk4::Orientation::Horizontal,
            0.0,
            f64::from(RGB_DENOISE_MAX_STRENGTH),
            1.0,
        );
        strength.set_widget_name("rgb-denoise-strength");
        strength.set_value(50.0);
        strength.set_draw_value(true);
        strength.set_hexpand(true);
        root.append(&row("Strength", &strength));
        let gamut =
            gtk4::DropDown::from_strings(&["Convert to working gamut", "Preserve wide gamut"]);
        gamut.set_widget_name("rgb-denoise-gamut");
        root.append(&row("Gamut", &gamut));
        let shadows = gtk4::DropDown::from_strings(&["Disabled", "Protect deep shadows"]);
        shadows.set_widget_name("rgb-denoise-shadows");
        root.append(&row("Shadows", &shadows));
        let detail = gtk4::DropDown::from_strings(&["Disabled", "Recover detail"]);
        detail.set_widget_name("rgb-denoise-detail");
        root.append(&row("Detail", &detail));
        let detail_strength =
            gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 0.0, 100.0, 1.0);
        detail_strength.set_widget_name("rgb-denoise-detail-strength");
        detail_strength.set_draw_value(true);
        detail_strength.set_hexpand(true);
        root.append(&row("Detail strength", &detail_strength));

        let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 5);
        let preview = gtk4::Button::with_label("Preview");
        preview.set_widget_name("rgb-denoise-preview");
        let full = gtk4::Button::with_label("Run full");
        full.set_widget_name("rgb-denoise-full");
        let export = gtk4::Button::with_label("Export");
        export.set_widget_name("rgb-denoise-export");
        let cancel = gtk4::Button::with_label("Cancel");
        cancel.set_widget_name("rgb-denoise-cancel");
        actions.append(&preview);
        actions.append(&full);
        actions.append(&export);
        actions.append(&cancel);
        root.append(&actions);

        let progress = gtk4::ProgressBar::new();
        progress.set_widget_name("rgb-denoise-progress");
        progress.set_show_text(true);
        progress.set_accessible_role(gtk4::AccessibleRole::ProgressBar);
        root.append(&progress);
        let plan = gtk4::Label::new(Some("Plan: unavailable"));
        plan.set_widget_name("rgb-denoise-plan");
        plan.set_halign(gtk4::Align::Start);
        plan.set_wrap(true);
        root.append(&plan);
        let memory = gtk4::Label::new(Some("Memory: unavailable"));
        memory.set_widget_name("rgb-denoise-memory");
        memory.set_halign(gtk4::Align::Start);
        root.append(&memory);
        let status = gtk4::Label::new(Some(
            "RGB denoise service unavailable; no inference was started.",
        ));
        status.set_widget_name("rgb-denoise-status");
        status.set_halign(gtk4::Align::Start);
        status.set_wrap(true);
        status.set_accessible_role(gtk4::AccessibleRole::Status);
        root.append(&status);

        Self {
            root,
            model,
            provider,
            working_profile,
            model_profile,
            scale,
            tile,
            strength,
            gamut,
            shadows,
            detail,
            detail_strength,
            preview,
            full,
            export,
            cancel,
            progress,
            plan,
            memory,
            status,
            model_hashes: Rc::new(RefCell::new(Vec::new())),
            working_ids: Rc::new(RefCell::new(Vec::new())),
            model_ids: Rc::new(RefCell::new(Vec::new())),
            signal_guard: Rc::new(Cell::new(false)),
        }
    }

    #[must_use]
    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    pub fn set_state(&self, state: &RgbDenoiseViewModel) {
        self.signal_guard.set(true);
        self.set_models(state);
        self.set_providers(state);
        self.set_profiles(state);
        self.scale.set_selected(0);
        self.tile.set_selected(tile_index(state.tile_size()));
        self.strength.set_value(f64::from(state.strength()));
        self.gamut.set_selected(u32::from(
            state.gamut() == RgbDenoiseGamutPolicy::PreserveWideGamut,
        ));
        self.shadows.set_selected(u32::from(
            state.shadows() == RgbDenoiseShadowPolicy::ProtectDeepShadows,
        ));
        self.detail
            .set_selected(u32::from(state.detail() == RgbDenoiseDetailPolicy::Recover));
        self.detail_strength
            .set_value(f64::from(state.detail_strength()));
        self.signal_guard.set(false);

        if let Some(plan) = state.plan() {
            self.plan.set_text(&format!(
                "Plan: {} · {}×{} tile · {}",
                plan.identity(),
                plan.scale(),
                plan.tile_size(),
                plan.provider().label()
            ));
            self.memory.set_text(&format!(
                "Memory estimate: {}",
                format_bytes(plan.memory_bytes())
            ));
        } else {
            self.plan.set_text("Plan: unavailable");
            self.memory.set_text("Memory estimate: unavailable");
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
            RgbDenoiseStatus::Running { .. } | RgbDenoiseStatus::Cancelling
        );
        self.preview
            .set_sensitive(state.plan().is_some() && !running);
        self.full.set_sensitive(state.plan().is_some() && !running);
        self.export
            .set_sensitive(state.plan().is_some() && !running);
        self.cancel.set_sensitive(running);
        self.detail_strength
            .set_sensitive(state.detail() == RgbDenoiseDetailPolicy::Recover && !running);
        self.strength.set_sensitive(!running);
    }

    pub fn connect_action<F>(&self, callback: F)
    where
        F: Fn(RgbDenoiseAction) + 'static,
    {
        let callback: ActionHandler = Rc::new(callback);
        connect_dropdown(
            &self.model,
            Rc::clone(&callback),
            Rc::clone(&self.signal_guard),
            {
                let hashes = Rc::clone(&self.model_hashes);
                move |index| RgbDenoiseAction::SelectModel(hashes.borrow().get(index).cloned())
            },
        );
        connect_dropdown(
            &self.provider,
            Rc::clone(&callback),
            Rc::clone(&self.signal_guard),
            move |index| RgbDenoiseAction::SelectProvider(AiProvider::all().get(index).copied()),
        );
        connect_dropdown(
            &self.working_profile,
            Rc::clone(&callback),
            Rc::clone(&self.signal_guard),
            {
                let ids = Rc::clone(&self.working_ids);
                move |index| {
                    RgbDenoiseAction::SelectWorkingProfile(ids.borrow().get(index).cloned())
                }
            },
        );
        connect_dropdown(
            &self.model_profile,
            Rc::clone(&callback),
            Rc::clone(&self.signal_guard),
            {
                let ids = Rc::clone(&self.model_ids);
                move |index| RgbDenoiseAction::SelectModelProfile(ids.borrow().get(index).cloned())
            },
        );
        connect_dropdown(
            &self.scale,
            Rc::clone(&callback),
            Rc::clone(&self.signal_guard),
            move |index| RgbDenoiseAction::SetScale(RGB_DENOISE_SCALES[index]),
        );
        connect_dropdown(
            &self.tile,
            Rc::clone(&callback),
            Rc::clone(&self.signal_guard),
            move |index| RgbDenoiseAction::SetTile(RGB_DENOISE_TILES[index]),
        );
        let guard = Rc::clone(&self.signal_guard);
        let callback_for_strength = Rc::clone(&callback);
        self.strength.connect_value_changed(move |scale| {
            if !guard.get() {
                callback_for_strength(RgbDenoiseAction::SetStrength(
                    scale
                        .value()
                        .round()
                        .clamp(0.0, f64::from(RGB_DENOISE_MAX_STRENGTH)) as u8,
                ));
            }
        });
        connect_dropdown(
            &self.gamut,
            Rc::clone(&callback),
            Rc::clone(&self.signal_guard),
            move |index| {
                RgbDenoiseAction::SetGamut(if index == 1 {
                    RgbDenoiseGamutPolicy::PreserveWideGamut
                } else {
                    RgbDenoiseGamutPolicy::ConvertToWorking
                })
            },
        );
        connect_dropdown(
            &self.shadows,
            Rc::clone(&callback),
            Rc::clone(&self.signal_guard),
            move |index| {
                RgbDenoiseAction::SetShadows(if index == 1 {
                    RgbDenoiseShadowPolicy::ProtectDeepShadows
                } else {
                    RgbDenoiseShadowPolicy::Disabled
                })
            },
        );
        connect_dropdown(
            &self.detail,
            Rc::clone(&callback),
            Rc::clone(&self.signal_guard),
            move |index| {
                RgbDenoiseAction::SetDetail(if index == 1 {
                    RgbDenoiseDetailPolicy::Recover
                } else {
                    RgbDenoiseDetailPolicy::Disabled
                })
            },
        );
        let guard = Rc::clone(&self.signal_guard);
        let callback_for_strength = Rc::clone(&callback);
        self.detail_strength.connect_value_changed(move |scale| {
            if !guard.get() {
                callback_for_strength(RgbDenoiseAction::SetDetailStrength(
                    scale.value().round().clamp(0.0, 100.0) as u8,
                ));
            }
        });
        connect_button(
            &self.preview,
            Rc::clone(&callback),
            RgbDenoiseAction::Preview,
        );
        connect_button(&self.full, Rc::clone(&callback), RgbDenoiseAction::Full);
        connect_button(&self.export, Rc::clone(&callback), RgbDenoiseAction::Export);
        connect_button(&self.cancel, callback, RgbDenoiseAction::Cancel);
    }

    fn set_models(&self, state: &RgbDenoiseViewModel) {
        let options = state.snapshot().qualified_models().collect::<Vec<_>>();
        self.model_hashes
            .replace(options.iter().map(|option| option.hash().clone()).collect());
        set_model_dropdown(
            &self.model,
            &gtk4::StringList::new(
                &options
                    .iter()
                    .map(|option| option.label())
                    .collect::<Vec<_>>(),
            ),
            options
                .iter()
                .position(|option| Some(option.hash()) == state.model()),
        );
    }

    fn set_providers(&self, state: &RgbDenoiseViewModel) {
        let labels = state
            .snapshot()
            .providers()
            .iter()
            .map(|provider| provider.label())
            .collect::<Vec<_>>();
        let list = gtk4::StringList::new(&labels);
        self.provider.set_model(Some(&list));
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

    fn set_profiles(&self, state: &RgbDenoiseViewModel) {
        let working = state.snapshot().working_profiles();
        self.working_ids.replace(
            working
                .iter()
                .map(|profile| profile.id().to_owned())
                .collect(),
        );
        let working_list = gtk4::StringList::new(
            &working
                .iter()
                .map(super::model::RgbDenoiseProfileOption::label)
                .collect::<Vec<_>>(),
        );
        self.working_profile.set_model(Some(&working_list));
        self.working_profile.set_selected(
            state
                .working_profile()
                .and_then(|id| {
                    self.working_ids
                        .borrow()
                        .iter()
                        .position(|entry| entry == id)
                })
                .unwrap_or(0) as u32,
        );
        let models = state.snapshot().model_profiles();
        self.model_ids.replace(
            models
                .iter()
                .map(|profile| profile.id().to_owned())
                .collect(),
        );
        let model_list = gtk4::StringList::new(
            &models
                .iter()
                .map(super::model::RgbDenoiseProfileOption::label)
                .collect::<Vec<_>>(),
        );
        self.model_profile.set_model(Some(&model_list));
        self.model_profile.set_selected(
            state
                .model_profile()
                .and_then(|id| self.model_ids.borrow().iter().position(|entry| entry == id))
                .unwrap_or(0) as u32,
        );
    }
}

impl Default for RgbDenoisePanel {
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
    F: Fn(usize) -> RgbDenoiseAction + 'static,
{
    dropdown.connect_selected_notify(move |dropdown| {
        if !guard.get() {
            callback(action(dropdown.selected() as usize));
        }
    });
}

fn connect_button(button: &gtk4::Button, callback: ActionHandler, action: RgbDenoiseAction) {
    button.connect_clicked(move |_| callback(action.clone()));
}

fn tile_index(tile: u32) -> u32 {
    RGB_DENOISE_TILES
        .iter()
        .position(|candidate| *candidate == tile)
        .unwrap_or(1) as u32
}

fn status_text(state: &RgbDenoiseViewModel) -> String {
    match state.status() {
        RgbDenoiseStatus::Failed(error) => error.message(),
        RgbDenoiseStatus::Running { kind, .. } => {
            format!("{} running through the application service.", kind.label())
        }
        RgbDenoiseStatus::Cancelling => {
            "Cancellation requested; waiting for the service.".to_owned()
        }
        RgbDenoiseStatus::Cancelled => "RGB denoise job cancelled.".to_owned(),
        RgbDenoiseStatus::Completed { kind, .. } => {
            format!("{} completed; source edit remains unchanged.", kind.label())
        }
        RgbDenoiseStatus::Planning => "Planning bounded RGB denoise resources…".to_owned(),
        RgbDenoiseStatus::Ready => "Ready; choose preview, full render, or export.".to_owned(),
        RgbDenoiseStatus::Idle => "Select a qualified model and profiles.".to_owned(),
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

fn set_model_dropdown(
    dropdown: &gtk4::DropDown,
    model: &gtk4::StringList,
    selected: Option<usize>,
) {
    dropdown.set_model(Some(model));
    dropdown.set_selected(selected.unwrap_or(0) as u32);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_models::ModelHash;
    use crate::rgb_denoise::RgbDenoiseFailure;

    #[test]
    fn bounded_controls_and_status_copy_are_stable() {
        assert_eq!(RGB_DENOISE_SCALES, [1]);
        assert_eq!(RGB_DENOISE_TILES, [128, 256, 512]);
        assert_eq!(format_bytes(1024 * 1024), "1.0 MiB");
        let failure = RgbDenoiseFailure::ProviderUnavailable;
        assert!(failure.message().contains("qualified provider"));
        let _ = ModelHash::new("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
            .expect("hash");
    }
}
