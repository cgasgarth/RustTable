//! GTK4 view for the mask-manager service projection.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::accessible::Property;
use gtk4::prelude::*;

use super::model::{
    MASK_MANAGER_MAX_FEATHER, MaskCombination, MaskManagerAction, MaskManagerCapability,
    MaskManagerSnapshot,
};

type ActionHandler = Rc<dyn Fn(MaskManagerAction)>;

#[derive(Clone)]
pub struct MaskManagerPanel {
    root: gtk4::Box,
    group: gtk4::DropDown,
    create_group: gtk4::Button,
    invert: gtk4::CheckButton,
    feather: gtk4::Scale,
    opacity: gtk4::Scale,
    combination: gtk4::DropDown,
    consumption: gtk4::Label,
    refresh: gtk4::Button,
    status: gtk4::Label,
    group_ids: Rc<RefCell<Vec<Option<String>>>>,
    signal_guard: Rc<Cell<bool>>,
}

impl MaskManagerPanel {
    #[must_use]
    pub fn new() -> Self {
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 5);
        root.set_widget_name("mask-manager");
        root.set_margin_top(6);
        root.set_margin_bottom(6);
        root.set_margin_start(6);
        root.set_margin_end(6);
        root.set_accessible_role(gtk4::AccessibleRole::Region);
        root.update_property(&[Property::Label("Mask manager")]);

        let heading = gtk4::Label::new(Some("mask manager"));
        heading.set_halign(gtk4::Align::Start);
        heading.add_css_class("title-4");
        root.append(&heading);

        let group = gtk4::DropDown::from_strings(&["No mask group"]);
        identify(&group, "mask-manager-group", "Select mask group");
        root.append(&row("Mask group", &group));
        let create_group = gtk4::Button::with_label("Create group");
        identify(
            &create_group,
            "mask-manager-create-group",
            "Create mask group through the mask service",
        );
        root.append(&create_group);

        let invert = gtk4::CheckButton::with_label("Invert mask");
        identify(&invert, "mask-manager-invert", "Invert selected mask group");
        root.append(&invert);

        let feather = gtk4::Scale::with_range(
            gtk4::Orientation::Horizontal,
            0.0,
            MASK_MANAGER_MAX_FEATHER,
            0.1,
        );
        feather.set_draw_value(true);
        feather.set_hexpand(true);
        identify(&feather, "mask-manager-feather", "Mask feathering amount");
        root.append(&row("Feather", &feather));

        let opacity = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 0.0, 1.0, 0.01);
        opacity.set_draw_value(true);
        opacity.set_digits(2);
        opacity.set_hexpand(true);
        identify(&opacity, "mask-manager-opacity", "Mask opacity");
        root.append(&row("Opacity", &opacity));

        let combination =
            gtk4::DropDown::from_strings(&["Union", "Intersection", "Difference", "Exclusion"]);
        identify(
            &combination,
            "mask-manager-combination",
            "Mask combination mode",
        );
        root.append(&row("Combination", &combination));

        let consumption = gtk4::Label::new(Some("Consumption: unavailable"));
        consumption.set_halign(gtk4::Align::Start);
        consumption.set_wrap(true);
        identify(
            &consumption,
            "mask-manager-consumption",
            "Cross-operation mask consumption state",
        );
        root.append(&consumption);

        let refresh = gtk4::Button::with_label("Refresh");
        identify(
            &refresh,
            "mask-manager-refresh",
            "Refresh mask-manager state",
        );
        root.append(&refresh);

        let status = gtk4::Label::new(Some("mask-manager service unavailable"));
        status.set_halign(gtk4::Align::Start);
        status.set_wrap(true);
        status.add_css_class("dim-label");
        status.set_accessible_role(gtk4::AccessibleRole::Status);
        identify(&status, "mask-manager-status", "Mask-manager status");
        root.append(&status);

        let panel = Self {
            root,
            group,
            create_group,
            invert,
            feather,
            opacity,
            combination,
            consumption,
            refresh,
            status,
            group_ids: Rc::new(RefCell::new(vec![None])),
            signal_guard: Rc::new(Cell::new(false)),
        };
        panel.set_state(&MaskManagerSnapshot::unavailable(
            0,
            "mask-manager backend capability is unavailable",
        ));
        panel
    }

    #[must_use]
    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    pub fn set_state(&self, state: &MaskManagerSnapshot) {
        self.signal_guard.set(true);
        let labels = if state.groups().is_empty() {
            vec!["No mask group".to_owned()]
        } else {
            state
                .groups()
                .iter()
                .map(|group| group.label().to_owned())
                .collect::<Vec<_>>()
        };
        self.group.set_model(Some(&gtk4::StringList::new(
            &labels.iter().map(String::as_str).collect::<Vec<_>>(),
        )));
        self.group_ids.replace(if state.groups().is_empty() {
            vec![None]
        } else {
            state
                .groups()
                .iter()
                .map(|group| Some(group.id().to_owned()))
                .collect()
        });
        self.group.set_selected(selected_group_index(state));
        self.invert.set_active(state.inverted());
        self.feather.set_value(state.feather());
        self.opacity.set_value(state.opacity());
        let combination_index = MaskCombination::all()
            .iter()
            .position(|mode| *mode == state.combination())
            .unwrap_or(0);
        self.combination
            .set_selected(u32::try_from(combination_index).unwrap_or(0));
        self.signal_guard.set(false);

        let available = state.capability().is_available();
        for widget in [
            self.group.clone().upcast::<gtk4::Widget>(),
            self.create_group.clone().upcast::<gtk4::Widget>(),
            self.invert.clone().upcast::<gtk4::Widget>(),
            self.feather.clone().upcast::<gtk4::Widget>(),
            self.opacity.clone().upcast::<gtk4::Widget>(),
            self.combination.clone().upcast::<gtk4::Widget>(),
        ] {
            widget.set_sensitive(available);
        }
        self.refresh.set_sensitive(true);
        self.consumption.set_text(&consumption_text(state));
        self.status.set_text(&status_text(state));
    }

    pub fn connect_action<F>(&self, callback: F)
    where
        F: Fn(MaskManagerAction) + 'static,
    {
        let callback: ActionHandler = Rc::new(callback);
        connect_dropdown(
            &self.group,
            Rc::clone(&callback),
            Rc::clone(&self.signal_guard),
            {
                let ids = Rc::clone(&self.group_ids);
                move |index| {
                    MaskManagerAction::SelectGroup(ids.borrow().get(index).cloned().flatten())
                }
            },
        );
        connect_dropdown(
            &self.combination,
            Rc::clone(&callback),
            Rc::clone(&self.signal_guard),
            move |index| {
                MaskManagerAction::SetCombination(
                    MaskCombination::all()
                        .get(index)
                        .copied()
                        .unwrap_or_default(),
                )
            },
        );
        {
            let callback = Rc::clone(&callback);
            self.create_group
                .connect_clicked(move |_| callback(MaskManagerAction::CreateGroup));
        }
        {
            let callback = Rc::clone(&callback);
            let guard = Rc::clone(&self.signal_guard);
            self.invert.connect_toggled(move |button| {
                if !guard.get() {
                    callback(MaskManagerAction::SetInverted(button.is_active()));
                }
            });
        }
        connect_scale(
            &self.feather,
            Rc::clone(&callback),
            Rc::clone(&self.signal_guard),
            MaskManagerAction::SetFeather,
        );
        connect_scale(
            &self.opacity,
            Rc::clone(&callback),
            Rc::clone(&self.signal_guard),
            MaskManagerAction::SetOpacity,
        );
        {
            let callback = Rc::clone(&callback);
            self.refresh
                .connect_clicked(move |_| callback(MaskManagerAction::Refresh));
        }
    }
}

impl Default for MaskManagerPanel {
    fn default() -> Self {
        Self::new()
    }
}

fn selected_group_index(state: &MaskManagerSnapshot) -> u32 {
    state
        .selected_group()
        .and_then(|selected| {
            state
                .groups()
                .iter()
                .position(|group| group.id() == selected)
        })
        .and_then(|index| u32::try_from(index).ok())
        .unwrap_or(0)
}

fn consumption_text(state: &MaskManagerSnapshot) -> String {
    match state.consumption() {
        super::model::MaskConsumptionState::NotConsumed => "Consumption: not consumed".to_owned(),
        super::model::MaskConsumptionState::ConsumedBy(operation) => {
            format!("Consumption: used by {operation}")
        }
        super::model::MaskConsumptionState::Unavailable { reason } => {
            format!("Consumption: unavailable · {reason}")
        }
    }
}

fn status_text(state: &MaskManagerSnapshot) -> String {
    match state.capability() {
        MaskManagerCapability::Available => {
            format!("Mask manager ready · generation {}", state.generation())
        }
        MaskManagerCapability::Unavailable { reason } => {
            format!("Mask manager unavailable · {reason}")
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
    F: Fn(usize) -> MaskManagerAction + 'static,
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

fn connect_scale<F>(scale: &gtk4::Scale, callback: ActionHandler, guard: Rc<Cell<bool>>, action: F)
where
    F: Fn(f64) -> MaskManagerAction + 'static,
{
    scale.connect_value_changed(move |scale| {
        if !guard.get() {
            callback(action(scale.value()));
        }
    });
}
