//! GTK4 view for the multiscale-retouch service projection.

use std::cell::Cell;
use std::rc::Rc;

use gtk4::accessible::Property;
use gtk4::prelude::*;

use super::model::{
    MULTISCALE_RETOUCH_BANDS, MULTISCALE_RETOUCH_MAX_STRENGTH, MultiscaleBand,
    MultiscaleRetouchAction, MultiscaleRetouchSnapshot, MultiscaleRetouchStatus,
    MultiscaleSourceTarget,
};

type ActionHandler = Rc<dyn Fn(MultiscaleRetouchAction)>;

#[derive(Clone)]
pub struct MultiscaleRetouchPanel {
    root: gtk4::Box,
    band: gtk4::DropDown,
    source: gtk4::DropDown,
    target: gtk4::DropDown,
    strength: gtk4::Scale,
    preview: gtk4::Button,
    cancel: gtk4::Button,
    refresh: gtk4::Button,
    progress: gtk4::ProgressBar,
    status: gtk4::Label,
    signal_guard: Rc<Cell<bool>>,
}

impl MultiscaleRetouchPanel {
    #[must_use]
    pub fn new() -> Self {
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 5);
        root.set_widget_name("multiscale-retouch");
        root.set_margin_top(6);
        root.set_margin_bottom(6);
        root.set_margin_start(6);
        root.set_margin_end(6);
        root.set_accessible_role(gtk4::AccessibleRole::Region);
        root.update_property(&[Property::Label("Multiscale retouch")]);

        let heading = gtk4::Label::new(Some("multiscale retouch"));
        heading.set_halign(gtk4::Align::Start);
        heading.add_css_class("title-4");
        root.append(&heading);

        let band = gtk4::DropDown::from_strings(&[
            "Original image",
            "Band 1",
            "Band 2",
            "Band 3",
            "Band 4",
            "Band 5",
            "Residual",
        ]);
        identify(&band, "multiscale-retouch-band", "Wavelet scale or band");
        root.append(&row("Band", &band));

        let source = gtk4::DropDown::from_strings(&["Source", "Target"]);
        identify(&source, "multiscale-retouch-source", "Retouch source");
        root.append(&row("Source", &source));
        let target = gtk4::DropDown::from_strings(&["Source", "Target"]);
        identify(&target, "multiscale-retouch-target", "Retouch target");
        root.append(&row("Target", &target));

        let strength = gtk4::Scale::with_range(
            gtk4::Orientation::Horizontal,
            0.0,
            f64::from(MULTISCALE_RETOUCH_MAX_STRENGTH),
            1.0,
        );
        strength.set_value(50.0);
        strength.set_draw_value(true);
        strength.set_hexpand(true);
        identify(&strength, "multiscale-retouch-strength", "Retouch strength");
        root.append(&row("Strength", &strength));

        let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 5);
        let preview = gtk4::Button::with_label("Preview");
        identify(
            &preview,
            "multiscale-retouch-preview",
            "Start retouch preview",
        );
        let cancel = gtk4::Button::with_label("Cancel");
        identify(
            &cancel,
            "multiscale-retouch-cancel",
            "Cancel retouch preview",
        );
        let refresh = gtk4::Button::with_label("Refresh");
        identify(
            &refresh,
            "multiscale-retouch-refresh",
            "Refresh retouch capability",
        );
        actions.append(&preview);
        actions.append(&cancel);
        actions.append(&refresh);
        root.append(&actions);

        let progress = gtk4::ProgressBar::new();
        progress.set_show_text(true);
        progress.set_widget_name("multiscale-retouch-progress");
        progress.set_accessible_role(gtk4::AccessibleRole::ProgressBar);
        progress.update_property(&[Property::Label("Multiscale retouch progress")]);
        root.append(&progress);

        let status = gtk4::Label::new(Some("multiscale-retouch service unavailable"));
        status.set_halign(gtk4::Align::Start);
        status.set_wrap(true);
        status.add_css_class("dim-label");
        status.set_accessible_role(gtk4::AccessibleRole::Status);
        identify(
            &status,
            "multiscale-retouch-status",
            "Multiscale retouch status",
        );
        root.append(&status);

        let panel = Self {
            root,
            band,
            source,
            target,
            strength,
            preview,
            cancel,
            refresh,
            progress,
            status,
            signal_guard: Rc::new(Cell::new(false)),
        };
        panel.set_state(&MultiscaleRetouchSnapshot::unavailable(
            0,
            "multiscale-retouch backend capability is unavailable",
        ));
        panel
    }

    #[must_use]
    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    pub fn set_state(&self, state: &MultiscaleRetouchSnapshot) {
        self.signal_guard.set(true);
        self.band.set_selected(band_index(state.band()));
        self.source
            .set_selected(u32::from(state.source() == MultiscaleSourceTarget::Target));
        self.target
            .set_selected(u32::from(state.target() == MultiscaleSourceTarget::Target));
        self.strength.set_value(f64::from(state.strength()));
        self.signal_guard.set(false);

        let available = state.capability().is_available();
        for widget in [
            self.band.clone().upcast::<gtk4::Widget>(),
            self.source.clone().upcast::<gtk4::Widget>(),
            self.target.clone().upcast::<gtk4::Widget>(),
            self.strength.clone().upcast::<gtk4::Widget>(),
        ] {
            widget.set_sensitive(available && !is_running(state));
        }
        self.preview.set_sensitive(available && !is_running(state));
        self.cancel.set_sensitive(is_running(state));
        self.refresh.set_sensitive(true);
        if let Some(progress) = state.progress() {
            self.progress.set_fraction(progress.fraction());
            self.progress.set_text(Some(&format!(
                "{} / {}",
                progress.completed(),
                progress.total()
            )));
        } else {
            self.progress.set_fraction(0.0);
            self.progress.set_text(Some("No active job"));
        }
        self.status.set_text(&status_text(state));
    }

    pub fn connect_action<F>(&self, callback: F)
    where
        F: Fn(MultiscaleRetouchAction) + 'static,
    {
        let callback: ActionHandler = Rc::new(callback);
        connect_dropdown(
            &self.band,
            Rc::clone(&callback),
            Rc::clone(&self.signal_guard),
            move |index| {
                MultiscaleRetouchAction::SetBand(
                    MultiscaleBand::all()
                        .get(index)
                        .copied()
                        .unwrap_or(MultiscaleBand::Original),
                )
            },
        );
        connect_dropdown(
            &self.source,
            Rc::clone(&callback),
            Rc::clone(&self.signal_guard),
            |index| MultiscaleRetouchAction::SetSource(source_target(index)),
        );
        connect_dropdown(
            &self.target,
            Rc::clone(&callback),
            Rc::clone(&self.signal_guard),
            |index| MultiscaleRetouchAction::SetTarget(source_target(index)),
        );
        {
            let callback = Rc::clone(&callback);
            let guard = Rc::clone(&self.signal_guard);
            self.strength.connect_value_changed(move |scale| {
                if !guard.get() {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    callback(MultiscaleRetouchAction::SetStrength(
                        scale.value().round() as u8
                    ));
                }
            });
        }
        {
            let callback = Rc::clone(&callback);
            self.preview
                .connect_clicked(move |_| callback(MultiscaleRetouchAction::Preview));
        }
        {
            let callback = Rc::clone(&callback);
            self.cancel
                .connect_clicked(move |_| callback(MultiscaleRetouchAction::Cancel));
        }
        self.refresh
            .connect_clicked(move |_| callback(MultiscaleRetouchAction::Refresh));
    }
}

impl Default for MultiscaleRetouchPanel {
    fn default() -> Self {
        Self::new()
    }
}

fn band_index(band: MultiscaleBand) -> u32 {
    match band {
        MultiscaleBand::Original => 0,
        MultiscaleBand::Band(value) => MULTISCALE_RETOUCH_BANDS
            .iter()
            .position(|candidate| *candidate == value)
            .and_then(|index| u32::try_from(index + 1).ok())
            .unwrap_or(0),
        MultiscaleBand::Residual => u32::try_from(MULTISCALE_RETOUCH_BANDS.len() + 1).unwrap_or(0),
    }
}

fn source_target(index: usize) -> MultiscaleSourceTarget {
    if index == 1 {
        MultiscaleSourceTarget::Target
    } else {
        MultiscaleSourceTarget::Source
    }
}

fn is_running(state: &MultiscaleRetouchSnapshot) -> bool {
    matches!(
        state.status(),
        MultiscaleRetouchStatus::Running { .. } | MultiscaleRetouchStatus::Cancelling { .. }
    )
}

fn status_text(state: &MultiscaleRetouchSnapshot) -> String {
    match state.status() {
        MultiscaleRetouchStatus::Unavailable => {
            "Multiscale retouch unavailable; no processing was started.".to_owned()
        }
        MultiscaleRetouchStatus::Ready => format!(
            "Multiscale retouch ready · generation {}",
            state.generation()
        ),
        MultiscaleRetouchStatus::Running { job } => {
            format!("Multiscale retouch running · job {job}")
        }
        MultiscaleRetouchStatus::Cancelling { job } => {
            format!("Multiscale retouch cancelling · job {job}")
        }
        MultiscaleRetouchStatus::Completed { job } => {
            format!("Multiscale retouch completed · job {job}")
        }
        MultiscaleRetouchStatus::Cancelled { job } => {
            format!("Multiscale retouch cancelled · job {job}")
        }
        MultiscaleRetouchStatus::Failed { message } => {
            format!("Multiscale retouch failed · {message}")
        }
    }
}

fn row(label: &str, widget: &impl IsA<gtk4::Widget>) -> gtk4::Box {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 5);
    let label_widget = gtk4::Label::new(Some(label));
    label_widget.set_halign(gtk4::Align::Start);
    label_widget.set_width_chars(13);
    row.append(&label_widget);
    row.append(widget);
    row
}

fn identify(widget: &impl IsA<gtk4::Widget>, id: &str, label: &str) {
    widget.set_widget_name(id);
    widget.set_tooltip_text(Some(label));
}

fn connect_dropdown<F>(
    dropdown: &gtk4::DropDown,
    callback: ActionHandler,
    guard: Rc<Cell<bool>>,
    action: F,
) where
    F: Fn(usize) -> MultiscaleRetouchAction + 'static,
{
    dropdown.connect_selected_notify(move |dropdown| {
        if guard.get() {
            return;
        }
        let Ok(index) = usize::try_from(dropdown.selected()) else {
            return;
        };
        callback(action(index));
    });
}
