//! Darktable-shaped GTK4 darkroom composition.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::accessible::Property;
use gtk4::gdk;
use gtk4::prelude::*;
use rusttable_core::{PhotoId, Revision};

use super::{ExposurePanel, PhotoPreview, ThemeRole, apply_theme_role};
use crate::presentation::PhotoDetailViewModel;
use crate::viewport_presentation::{
    DarkroomViewportAction, DarkroomViewportCommand, DarkroomViewportState, DarkroomZoom,
    ViewportColorMode, ViewportComparison, ViewportGeneration,
};

/// Stable widget identifiers for the initial darkroom surface.
pub const DARKROOM_WIDGET_IDS: [&str; 13] = [
    "darkroom-page",
    "darkroom-toolbar-top",
    "darkroom-photo-preview",
    "darkroom-toolbar-bottom",
    "darkroom-left-panel",
    "darkroom-navigation",
    "darkroom-snapshots",
    "darkroom-history",
    "darkroom-image-information",
    "darkroom-right-panel",
    "darkroom-histogram",
    "darkroom-module-groups",
    "exposure",
];

/// Stable left-to-right focus order for the darkroom rail controls.
pub const DARKROOM_RAIL_FOCUS_ORDER: [&str; 8] = [
    "darkroom-navigation",
    "darkroom-snapshots",
    "darkroom-history",
    "darkroom-image-information",
    "group-active",
    "group-favorites",
    "group-technical",
    "group-grading",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DarkroomModuleGroup {
    Active,
    Favorites,
    Technical,
    Grading,
}

impl DarkroomModuleGroup {
    pub(super) fn matches_title(self, title: &str) -> bool {
        let title = title.to_ascii_lowercase();
        match self {
            Self::Active => true,
            Self::Favorites => title.contains("favorite"),
            Self::Technical => ["balance", "denoise", "lens", "raw", "sharpen"]
                .iter()
                .any(|term| title.contains(term)),
            Self::Grading => ["color", "contrast", "curve", "exposure", "tone"]
                .iter()
                .any(|term| title.contains(term)),
        }
    }
}

type DarkroomModuleGroupHandler = Box<dyn Fn(DarkroomModuleGroup)>;

/// Stable identifiers for the darkroom viewport controls and filmstrip boundary.
pub const DARKROOM_VIEWPORT_WIDGET_IDS: [&str; 9] = [
    "darkroom-viewport",
    "darkroom-soft-proof",
    "darkroom-gamut-check",
    "darkroom-zoom",
    "darkroom-fit",
    "darkroom-before-after",
    "darkroom-viewport-projection",
    "darkroom-filmstrip-boundary",
    "darkroom-image-canvas",
];

/// Focus order for all controls introduced by the darkroom viewport batch.
pub const DARKROOM_VIEWPORT_FOCUS_ORDER: [&str; 5] = [
    "darkroom-soft-proof",
    "darkroom-gamut-check",
    "darkroom-zoom",
    "darkroom-fit",
    "darkroom-before-after",
];

/// Application-owned receiver for viewport commands. The orchestrator supplies the renderer or
/// controller; the GTK view only emits typed, generation-tagged intent.
pub type DarkroomViewportActionHandler = Box<dyn Fn(DarkroomViewportCommand)>;

/// Native GTK widgets owned by the darkroom view.
#[derive(Clone)]
pub struct DarkroomView {
    page: gtk4::Box,
    preview: PhotoPreview,
    viewport_state: Rc<RefCell<DarkroomViewportState>>,
    viewport_controls: ViewportControls,
    viewport_handler: Rc<RefCell<Option<DarkroomViewportActionHandler>>>,
    left_panel: gtk4::Box,
    left_modules: gtk4::Box,
    right_panel: gtk4::Box,
    right_modules: gtk4::Box,
    exposure: ExposurePanel,
    rail_status: DarkroomRailStatus,
    histogram: gtk4::Stack,
    module_group: Rc<Cell<DarkroomModuleGroup>>,
    module_group_handler: Rc<RefCell<Option<DarkroomModuleGroupHandler>>>,
}

impl DarkroomView {
    /// Builds the initial Darktable darkroom around the immutable preview boundary.
    #[must_use]
    pub fn new(panel_width: i32) -> Self {
        debug_assert_eq!(DARKROOM_RAIL_FOCUS_ORDER.len(), 8);
        let preview = PhotoPreview::new();
        let viewport_state = Rc::new(RefCell::new(DarkroomViewportState::default()));
        let viewport_handler = Rc::new(RefCell::new(None));
        let (page, viewport_controls) = darkroom_page(&preview, &viewport_state, &viewport_handler);
        let (left_panel, left_modules, rail_status) = left_panel(panel_width);
        let (right_panel, right_modules, exposure, histogram, module_group, module_group_handler) =
            right_panel(panel_width);
        Self {
            page,
            preview,
            viewport_state,
            viewport_controls,
            viewport_handler,
            left_panel,
            left_modules,
            right_panel,
            right_modules,
            exposure,
            rail_status,
            histogram,
            module_group,
            module_group_handler,
        }
    }

    #[must_use]
    pub fn page(&self) -> &gtk4::Box {
        &self.page
    }

    #[must_use]
    pub fn preview(&self) -> &PhotoPreview {
        &self.preview
    }

    /// Returns the current display-free viewport state.
    #[must_use]
    pub fn viewport_state(&self) -> DarkroomViewportState {
        *self.viewport_state.borrow()
    }

    /// Starts a new generation for the selected catalog photo/edit projection.
    pub fn set_viewport_selection(
        &self,
        photo_id: PhotoId,
        edit_revision: Revision,
        generation: ViewportGeneration,
    ) {
        self.viewport_state
            .borrow_mut()
            .select(photo_id, edit_revision, generation);
        self.sync_viewport_projection();
    }

    /// Restores truthful no-photo state and resets transient viewport controls.
    pub fn clear_viewport_selection(&self) {
        self.viewport_state.borrow_mut().clear_selection();
        self.sync_viewport_projection();
    }

    /// Connects typed toolbar and navigation commands to the application orchestrator.
    pub fn connect_viewport_action<F>(&self, handler: F)
    where
        F: Fn(DarkroomViewportCommand) + 'static,
    {
        self.viewport_handler.replace(Some(Box::new(handler)));
    }

    /// Reapplies the current projection after the orchestrator installs a new texture.
    pub fn sync_viewport_projection(&self) {
        sync_viewport_controls(&self.viewport_controls, &self.preview, &self.viewport_state);
    }

    #[must_use]
    pub fn left_panel(&self) -> &gtk4::Box {
        &self.left_panel
    }

    #[must_use]
    pub fn left_modules(&self) -> &gtk4::Box {
        &self.left_modules
    }

    #[must_use]
    pub fn right_panel(&self) -> &gtk4::Box {
        &self.right_panel
    }

    #[must_use]
    pub fn right_modules(&self) -> &gtk4::Box {
        &self.right_modules
    }

    #[must_use]
    pub fn exposure(&self) -> &ExposurePanel {
        &self.exposure
    }

    pub(super) fn module_group_state(&self) -> Rc<Cell<DarkroomModuleGroup>> {
        Rc::clone(&self.module_group)
    }

    pub(super) fn connect_module_group<F>(&self, handler: F)
    where
        F: Fn(DarkroomModuleGroup) + 'static,
    {
        self.module_group_handler.replace(Some(Box::new(handler)));
    }

    /// Projects a selected image into the side-rail states without inventing unavailable data.
    pub fn set_detail(&self, detail: &PhotoDetailViewModel) {
        self.rail_status
            .navigation
            .set_text("filmstrip navigation ready");
        self.rail_status
            .snapshots
            .set_text("snapshot data unavailable");
        self.rail_status
            .history
            .set_text("edit history unavailable");
        self.rail_status.image_information.set_text(&format!(
            "{} · {} metadata fields",
            detail.title().as_str(),
            detail.facts().count()
        ));
        self.histogram.set_visible_child_name("unavailable");
    }

    /// Restores the explicit no-selection state of every side-rail surface.
    pub fn clear_detail(&self) {
        self.rail_status
            .navigation
            .set_text("select a photo to navigate");
        self.rail_status
            .snapshots
            .set_text("select a photo to view snapshots");
        self.rail_status
            .history
            .set_text("select a photo to view edit history");
        self.rail_status
            .image_information
            .set_text("image information unavailable");
        self.histogram.set_visible_child_name("empty");
    }
}

#[derive(Clone)]
struct DarkroomRailStatus {
    navigation: gtk4::Label,
    snapshots: gtk4::Label,
    history: gtk4::Label,
    image_information: gtk4::Label,
}

fn darkroom_page(
    preview: &PhotoPreview,
    state: &Rc<RefCell<DarkroomViewportState>>,
    handler: &Rc<RefCell<Option<DarkroomViewportActionHandler>>>,
) -> (gtk4::Box, ViewportControls) {
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    page.set_widget_name("darkroom-page");
    page.set_hexpand(true);
    page.set_vexpand(true);
    apply_theme_role(&page, ThemeRole::Darkroom);

    page.set_focusable(true);

    let projection = gtk4::Label::new(Some("no photo selected"));
    projection.set_widget_name(DARKROOM_VIEWPORT_WIDGET_IDS[6]);
    projection.set_halign(gtk4::Align::End);
    projection.set_valign(gtk4::Align::Start);
    projection.set_margin_top(8);
    projection.set_margin_end(10);
    projection.add_css_class("dim-label");
    projection.set_accessible_role(gtk4::AccessibleRole::Status);
    projection.update_property(&[Property::Label("Current viewport projection")]);

    let zoom = gtk4::DropDown::from_strings(&DarkroomZoom::ALL.map(DarkroomZoom::label));
    zoom.set_widget_name(DARKROOM_VIEWPORT_WIDGET_IDS[3]);
    zoom.set_tooltip_text(Some("Image zoom level"));
    zoom.update_property(&[Property::Label("Image zoom level")]);

    let fit = chrome_button(
        DARKROOM_VIEWPORT_WIDGET_IDS[4],
        "fit",
        "Fit image to viewport",
    );
    let before_after = chrome_toggle(
        DARKROOM_VIEWPORT_WIDGET_IDS[5],
        "before/after",
        "Compare before and after",
    );
    let soft_proof = chrome_toggle(
        DARKROOM_VIEWPORT_WIDGET_IDS[1],
        "soft proof",
        "Toggle soft proof",
    );
    let gamut_check = chrome_toggle(
        DARKROOM_VIEWPORT_WIDGET_IDS[2],
        "gamut check",
        "Toggle gamut warning",
    );
    let controls = ViewportControls {
        zoom: zoom.clone(),
        fit: fit.clone(),
        before_after: before_after.clone(),
        soft_proof: soft_proof.clone(),
        gamut_check: gamut_check.clone(),
        projection: projection.clone(),
        canvas: find_image_canvas(preview.widget().upcast_ref()),
        sync_guard: Rc::new(Cell::new(false)),
    };

    let top = toolbar("darkroom-toolbar-top", "Darkroom color proofing controls");
    top.append(&soft_proof);
    top.append(&gamut_check);

    let bottom = toolbar("darkroom-toolbar-bottom", "Darkroom viewport controls");
    bottom.append(&zoom);
    bottom.append(&fit);
    bottom.append(&before_after);

    let viewport = gtk4::Overlay::new();
    viewport.set_widget_name(DARKROOM_VIEWPORT_WIDGET_IDS[0]);
    viewport.set_hexpand(true);
    viewport.set_vexpand(true);
    viewport.set_accessible_role(gtk4::AccessibleRole::Group);
    viewport.update_property(&[Property::Label("Darkroom image viewport")]);
    viewport.set_child(Some(preview.widget()));
    viewport.add_overlay(&projection);

    let boundary = gtk4::Separator::new(gtk4::Orientation::Horizontal);
    boundary.set_widget_name(DARKROOM_VIEWPORT_WIDGET_IDS[7]);
    boundary.update_property(&[Property::Label("Filmstrip boundary")]);

    page.append(&top);
    page.append(&viewport);
    page.append(&bottom);
    page.append(&boundary);

    connect_viewport_controls(&controls, state, handler, preview);
    install_viewport_input(&page, preview, &controls, state, handler);
    debug_assert_eq!(DARKROOM_VIEWPORT_FOCUS_ORDER.len(), 5);
    sync_viewport_controls(&controls, preview, state);
    (page, controls)
}

#[derive(Clone)]
struct ViewportControls {
    zoom: gtk4::DropDown,
    fit: gtk4::Button,
    before_after: gtk4::ToggleButton,
    soft_proof: gtk4::ToggleButton,
    gamut_check: gtk4::ToggleButton,
    projection: gtk4::Label,
    canvas: Option<gtk4::Picture>,
    sync_guard: Rc<Cell<bool>>,
}

fn toolbar(id: &str, accessible_name: &str) -> gtk4::Box {
    let toolbar = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);
    toolbar.set_widget_name(id);
    toolbar.add_css_class("dt_darkroom_toolbar");
    toolbar.set_accessible_role(gtk4::AccessibleRole::Toolbar);
    toolbar.update_property(&[Property::Label(accessible_name)]);
    toolbar
}

fn chrome_toggle(id: &str, label: &str, accessible_name: &str) -> gtk4::ToggleButton {
    let button = gtk4::ToggleButton::with_label(label);
    button.set_widget_name(id);
    button.add_css_class("dt_button");
    button.set_focus_on_click(false);
    button.set_tooltip_text(Some(accessible_name));
    button.update_property(&[Property::Label(accessible_name)]);
    button
}

fn connect_viewport_controls(
    controls: &ViewportControls,
    state: &Rc<RefCell<DarkroomViewportState>>,
    handler: &Rc<RefCell<Option<DarkroomViewportActionHandler>>>,
    preview: &PhotoPreview,
) {
    let connect = |action: DarkroomViewportAction, controls: &ViewportControls| {
        let state = Rc::clone(state);
        let handler = Rc::clone(handler);
        let controls = controls.clone();
        let preview = preview.clone();
        move || dispatch_viewport_action(&state, &handler, &controls, &preview, action)
    };

    let fit_action = connect(DarkroomViewportAction::Fit, controls);
    controls.fit.connect_clicked(move |_| fit_action());

    let before_action = connect(DarkroomViewportAction::ToggleBeforeAfter, controls);
    controls
        .before_after
        .connect_toggled(move |_| before_action());

    let proof_action = connect(DarkroomViewportAction::ToggleSoftProof, controls);
    controls.soft_proof.connect_toggled(move |_| proof_action());

    let gamut_action = connect(DarkroomViewportAction::ToggleGamutCheck, controls);
    controls
        .gamut_check
        .connect_toggled(move |_| gamut_action());

    let state = Rc::clone(state);
    let handler = Rc::clone(handler);
    let controls_clone = controls.clone();
    let preview = preview.clone();
    controls.zoom.connect_selected_notify(move |zoom| {
        let Some(zoom) = usize::try_from(zoom.selected())
            .ok()
            .and_then(|index| DarkroomZoom::ALL.get(index).copied())
        else {
            return;
        };
        dispatch_viewport_action(
            &state,
            &handler,
            &controls_clone,
            &preview,
            DarkroomViewportAction::SetZoom(zoom),
        );
    });
}

fn install_viewport_input(
    page: &gtk4::Box,
    preview: &PhotoPreview,
    controls: &ViewportControls,
    state: &Rc<RefCell<DarkroomViewportState>>,
    handler: &Rc<RefCell<Option<DarkroomViewportActionHandler>>>,
) {
    install_viewport_keyboard(page, preview, controls, state, handler);
    install_viewport_scroll(preview, controls, state, handler);
    install_viewport_drag(preview, controls, state, handler);
}

fn install_viewport_keyboard(
    page: &gtk4::Box,
    preview: &PhotoPreview,
    controls: &ViewportControls,
    state: &Rc<RefCell<DarkroomViewportState>>,
    handler: &Rc<RefCell<Option<DarkroomViewportActionHandler>>>,
) {
    let key = gtk4::EventControllerKey::new();
    key.set_propagation_phase(gtk4::PropagationPhase::Capture);
    let state_for_key = Rc::clone(state);
    let handler_for_key = Rc::clone(handler);
    let controls_for_key = controls.clone();
    let preview_for_key = preview.clone();
    key.connect_key_pressed(move |_, key, _, _| {
        let action = match key {
            gdk::Key::Left => Some(DarkroomViewportAction::Pan {
                delta_x: -100,
                delta_y: 0,
            }),
            gdk::Key::Right => Some(DarkroomViewportAction::Pan {
                delta_x: 100,
                delta_y: 0,
            }),
            gdk::Key::Up => Some(DarkroomViewportAction::Pan {
                delta_x: 0,
                delta_y: -100,
            }),
            gdk::Key::Down => Some(DarkroomViewportAction::Pan {
                delta_x: 0,
                delta_y: 100,
            }),
            gdk::Key::plus | gdk::Key::KP_Add => Some(DarkroomViewportAction::ZoomIn),
            gdk::Key::minus | gdk::Key::KP_Subtract => Some(DarkroomViewportAction::ZoomOut),
            gdk::Key::_0 | gdk::Key::KP_0 => Some(DarkroomViewportAction::Fit),
            gdk::Key::b => Some(DarkroomViewportAction::ToggleBeforeAfter),
            gdk::Key::s => Some(DarkroomViewportAction::ToggleSoftProof),
            gdk::Key::g => Some(DarkroomViewportAction::ToggleGamutCheck),
            _ => None,
        };
        let Some(action) = action else {
            return gtk4::glib::Propagation::Proceed;
        };
        dispatch_viewport_action(
            &state_for_key,
            &handler_for_key,
            &controls_for_key,
            &preview_for_key,
            action,
        );
        gtk4::glib::Propagation::Stop
    });
    page.add_controller(key);
}

fn install_viewport_scroll(
    preview: &PhotoPreview,
    controls: &ViewportControls,
    state: &Rc<RefCell<DarkroomViewportState>>,
    handler: &Rc<RefCell<Option<DarkroomViewportActionHandler>>>,
) {
    let scroll = gtk4::EventControllerScroll::new(
        gtk4::EventControllerScrollFlags::VERTICAL | gtk4::EventControllerScrollFlags::HORIZONTAL,
    );
    let state_for_scroll = Rc::clone(state);
    let handler_for_scroll = Rc::clone(handler);
    let controls_for_scroll = controls.clone();
    let preview_for_scroll = preview.clone();
    scroll.connect_scroll(move |scroll, delta_x, delta_y| {
        let modifiers = scroll.current_event_state();
        let action = if modifiers
            .intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SUPER_MASK)
        {
            if delta_y.is_sign_negative() {
                DarkroomViewportAction::ZoomIn
            } else {
                DarkroomViewportAction::ZoomOut
            }
        } else {
            DarkroomViewportAction::Pan {
                delta_x: rounded_delta(-delta_x * 100.0),
                delta_y: rounded_delta(-delta_y * 100.0),
            }
        };
        dispatch_viewport_action(
            &state_for_scroll,
            &handler_for_scroll,
            &controls_for_scroll,
            &preview_for_scroll,
            action,
        );
        gtk4::glib::Propagation::Stop
    });
    preview.widget().add_controller(scroll);
}

fn install_viewport_drag(
    preview: &PhotoPreview,
    controls: &ViewportControls,
    state: &Rc<RefCell<DarkroomViewportState>>,
    handler: &Rc<RefCell<Option<DarkroomViewportActionHandler>>>,
) {
    let drag = gtk4::GestureDrag::new();
    let drag_delta = Rc::new(RefCell::new((0.0_f64, 0.0_f64)));
    let drag_delta_begin = Rc::clone(&drag_delta);
    drag.connect_drag_begin(move |_, _, _| {
        drag_delta_begin.replace((0.0, 0.0));
    });
    let state_for_drag = Rc::clone(state);
    let handler_for_drag = Rc::clone(handler);
    let controls_for_drag = controls.clone();
    let preview_for_drag = preview.clone();
    drag.connect_drag_update(move |_, offset_x, offset_y| {
        let mut previous = drag_delta.borrow_mut();
        let delta_x = rounded_delta(offset_x - previous.0);
        let delta_y = rounded_delta(offset_y - previous.1);
        previous.0 = offset_x;
        previous.1 = offset_y;
        if delta_x == 0 && delta_y == 0 {
            return;
        }
        dispatch_viewport_action(
            &state_for_drag,
            &handler_for_drag,
            &controls_for_drag,
            &preview_for_drag,
            DarkroomViewportAction::Pan { delta_x, delta_y },
        );
    });
    preview.widget().add_controller(drag);
}

fn dispatch_viewport_action(
    state: &Rc<RefCell<DarkroomViewportState>>,
    handler: &Rc<RefCell<Option<DarkroomViewportActionHandler>>>,
    controls: &ViewportControls,
    preview: &PhotoPreview,
    action: DarkroomViewportAction,
) {
    if controls.sync_guard.get() {
        return;
    }
    let generation = state.borrow().generation();
    let command = DarkroomViewportCommand::new(generation, action);
    if !state.borrow_mut().apply(command) {
        return;
    }
    sync_viewport_controls(controls, preview, state);
    if let Some(handler) = handler.borrow().as_ref() {
        handler(command);
    }
}

fn sync_viewport_controls(
    controls: &ViewportControls,
    preview: &PhotoPreview,
    state: &Rc<RefCell<DarkroomViewportState>>,
) {
    let state = *state.borrow();
    controls.sync_guard.set(true);
    controls.zoom.set_selected(state.zoom().index());
    controls
        .before_after
        .set_active(state.comparison() == ViewportComparison::Before);
    controls
        .soft_proof
        .set_active(state.color_mode() == ViewportColorMode::SoftProof);
    controls
        .gamut_check
        .set_active(state.color_mode() == ViewportColorMode::GamutCheck);
    controls.projection.set_text(&state.projection_label());
    apply_canvas_projection(controls.canvas.as_ref(), preview, state);
    controls.sync_guard.set(false);
}

fn apply_canvas_projection(
    canvas: Option<&gtk4::Picture>,
    preview: &PhotoPreview,
    state: DarkroomViewportState,
) {
    let Some(canvas) = canvas else {
        return;
    };
    let Some(percent) = state.zoom().percent() else {
        canvas.set_size_request(-1, -1);
        canvas.set_can_shrink(true);
        canvas.set_halign(gtk4::Align::Fill);
        canvas.set_valign(gtk4::Align::Fill);
        return;
    };
    let Some(texture) = preview.texture() else {
        return;
    };
    canvas.set_can_shrink(false);
    canvas.set_size_request(
        scaled_dimension(texture.width(), percent),
        scaled_dimension(texture.height(), percent),
    );
    canvas.set_halign(pan_alignment(state.pan().x()));
    canvas.set_valign(pan_alignment(state.pan().y()));
}

fn pan_alignment(value: i16) -> gtk4::Align {
    match value.cmp(&0) {
        std::cmp::Ordering::Less => gtk4::Align::Start,
        std::cmp::Ordering::Greater => gtk4::Align::End,
        std::cmp::Ordering::Equal => gtk4::Align::Center,
    }
}

fn scaled_dimension(value: i32, percent: u16) -> i32 {
    let scaled = (i64::from(value) * i64::from(percent) + 50) / 100;
    i32::try_from(scaled).unwrap_or(i32::MAX).max(1)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn rounded_delta(value: f64) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    value
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

fn find_image_canvas(widget: &gtk4::Widget) -> Option<gtk4::Picture> {
    if widget.widget_name() == "darkroom-image-canvas" {
        return widget.clone().downcast::<gtk4::Picture>().ok();
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(canvas) = find_image_canvas(&current) {
            return Some(canvas);
        }
        child = current.next_sibling();
    }
    None
}

fn left_panel(width: i32) -> (gtk4::Box, gtk4::Box, DarkroomRailStatus) {
    let panel = rail("darkroom-left-panel", width, "Darkroom left module rail");
    let modules = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    modules.set_widget_name("darkroom-left-modules");
    let (navigation, navigation_state) = rail_module(
        "darkroom-navigation",
        "navigation",
        true,
        "select a photo to navigate",
    );
    let (snapshots, snapshots_state) = rail_module(
        "darkroom-snapshots",
        "snapshots",
        false,
        "select a photo to view snapshots",
    );
    let (history, history_state) = rail_module(
        "darkroom-history",
        "history",
        false,
        "select a photo to view edit history",
    );
    let (image_information, image_information_state) = rail_module(
        "darkroom-image-information",
        "image information",
        false,
        "image information unavailable",
    );
    for module in [navigation, snapshots, history, image_information] {
        modules.append(&module);
    }
    let controller_modules = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    controller_modules.set_widget_name("darkroom-left-controller-modules");
    modules.append(&controller_modules);
    let scroll = gtk4::ScrolledWindow::builder()
        .child(&modules)
        .hexpand(true)
        .vexpand(true)
        .build();
    panel.append(&scroll);
    (
        panel,
        controller_modules,
        DarkroomRailStatus {
            navigation: navigation_state,
            snapshots: snapshots_state,
            history: history_state,
            image_information: image_information_state,
        },
    )
}

type DarkroomPanelBuild = (
    gtk4::Box,
    gtk4::Box,
    ExposurePanel,
    gtk4::Stack,
    Rc<Cell<DarkroomModuleGroup>>,
    Rc<RefCell<Option<DarkroomModuleGroupHandler>>>,
);

fn right_panel(width: i32) -> DarkroomPanelBuild {
    let panel = rail(
        "darkroom-right-panel",
        width,
        "Darkroom processing module rail",
    );
    let histogram = histogram();
    panel.append(&histogram);

    let groups = gtk4::Box::new(gtk4::Orientation::Horizontal, 1);
    groups.set_widget_name("darkroom-module-groups");
    groups.set_accessible_role(gtk4::AccessibleRole::Toolbar);
    groups.update_property(&[Property::Label("Processing module groups")]);
    let module_group = Rc::new(Cell::new(DarkroomModuleGroup::Active));
    let module_group_handler = Rc::new(RefCell::new(None));
    add_group_buttons(&groups, &module_group, &module_group_handler);
    panel.append(&groups);

    let modules = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    modules.set_widget_name("darkroom-right-modules");
    let exposure = ExposurePanel::new();
    modules.append(exposure.widget());
    let controller_modules = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    controller_modules.set_widget_name("darkroom-right-controller-modules");
    modules.append(&controller_modules);
    let scroll = gtk4::ScrolledWindow::builder()
        .child(&modules)
        .hexpand(true)
        .vexpand(true)
        .build();
    panel.append(&scroll);
    (
        panel,
        controller_modules,
        exposure,
        histogram,
        module_group,
        module_group_handler,
    )
}

fn histogram() -> gtk4::Stack {
    let histogram = gtk4::Stack::new();
    histogram.set_widget_name("darkroom-histogram");
    histogram.set_height_request(92);
    histogram.set_accessible_role(gtk4::AccessibleRole::Img);
    histogram.update_property(&[Property::Label("Image histogram")]);
    for (name, text) in [
        ("empty", "select a photo to show the histogram"),
        ("unavailable", "histogram unavailable for this preview"),
    ] {
        let state = gtk4::Label::new(Some(text));
        state.set_widget_name(&format!("darkroom-histogram-{name}"));
        state.set_halign(gtk4::Align::Center);
        state.set_valign(gtk4::Align::Center);
        state.add_css_class("dim-label");
        state.set_hexpand(true);
        state.set_vexpand(true);
        state.set_accessible_role(gtk4::AccessibleRole::Status);
        histogram.add_named(&state, Some(name));
    }
    histogram.set_visible_child_name("empty");
    histogram
}

fn add_group_buttons(
    groups: &gtk4::Box,
    state: &Rc<Cell<DarkroomModuleGroup>>,
    handler: &Rc<RefCell<Option<DarkroomModuleGroupHandler>>>,
) {
    let guard = Rc::new(Cell::new(false));
    let buttons = [
        (
            DarkroomModuleGroup::Active,
            "group-active",
            "●",
            "Active modules",
        ),
        (
            DarkroomModuleGroup::Favorites,
            "group-favorites",
            "★",
            "Favorite modules",
        ),
        (
            DarkroomModuleGroup::Technical,
            "group-technical",
            "○",
            "Technical modules",
        ),
        (
            DarkroomModuleGroup::Grading,
            "group-grading",
            "◐",
            "Grading modules",
        ),
    ]
    .into_iter()
    .map(|(group, id, icon, label)| {
        let button = chrome_toggle(id, icon, label);
        button.set_active(group == DarkroomModuleGroup::Active);
        let state = Rc::clone(state);
        let handler = Rc::clone(handler);
        let guard_for_callback = Rc::clone(&guard);
        button.connect_toggled(move |button| {
            if guard_for_callback.get() {
                return;
            }
            if !button.is_active() {
                guard_for_callback.set(true);
                button.set_active(true);
                guard_for_callback.set(false);
                return;
            }
            state.set(group);
            if let Some(handler) = handler.borrow().as_ref() {
                handler(group);
            }
        });
        button
    })
    .collect::<Vec<_>>();
    for button in buttons {
        groups.append(&button);
    }
}

fn rail(id: &str, width: i32, accessible_name: &str) -> gtk4::Box {
    let panel = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    panel.set_widget_name(id);
    panel.set_width_request(width);
    panel.set_accessible_role(gtk4::AccessibleRole::Group);
    panel.update_property(&[Property::Label(accessible_name)]);
    apply_theme_role(&panel, ThemeRole::Panel);
    panel
}

fn rail_module(
    id: &str,
    title: &str,
    initially_expanded: bool,
    state_text: &str,
) -> (gtk4::Expander, gtk4::Label) {
    let state = gtk4::Label::new(Some(state_text));
    state.set_widget_name(&format!("{id}-state"));
    state.set_halign(gtk4::Align::Start);
    state.add_css_class("dim-label");
    state.set_accessible_role(gtk4::AccessibleRole::Status);
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    content.append(&state);
    let expander = gtk4::Expander::builder()
        .label(title)
        .expanded(initially_expanded)
        .child(&content)
        .build();
    expander.set_widget_name(id);
    expander.set_focusable(true);
    expander.update_property(&[Property::Label(title)]);
    apply_theme_role(&expander, ThemeRole::ModuleGroup);
    (expander, state)
}

fn chrome_button(id: &str, label: &str, accessible_name: &str) -> gtk4::Button {
    let button = gtk4::Button::with_label(label);
    button.set_widget_name(id);
    button.add_css_class("dt_button");
    button.set_focus_on_click(false);
    button.update_property(&[Property::Label(accessible_name)]);
    button
}

#[cfg(test)]
mod tests {
    use super::{
        DARKROOM_RAIL_FOCUS_ORDER, DARKROOM_VIEWPORT_FOCUS_ORDER, DARKROOM_VIEWPORT_WIDGET_IDS,
        DARKROOM_WIDGET_IDS, DarkroomModuleGroup,
    };

    #[test]
    fn darkroom_contract_has_stable_unique_roles_and_initial_exposure() {
        let unique = DARKROOM_WIDGET_IDS
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), DARKROOM_WIDGET_IDS.len());
        assert_eq!(DARKROOM_WIDGET_IDS[0], "darkroom-page");
        assert_eq!(DARKROOM_WIDGET_IDS.last(), Some(&"exposure"));
        assert_eq!(DARKROOM_RAIL_FOCUS_ORDER[0], "darkroom-navigation");
        assert_eq!(DARKROOM_RAIL_FOCUS_ORDER.last(), Some(&"group-grading"));
    }

    #[test]
    fn viewport_controls_have_unique_accessible_focus_contract() {
        let unique = DARKROOM_VIEWPORT_WIDGET_IDS
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), DARKROOM_VIEWPORT_WIDGET_IDS.len());
        assert_eq!(DARKROOM_VIEWPORT_FOCUS_ORDER[0], "darkroom-soft-proof");
        assert!(
            DARKROOM_VIEWPORT_FOCUS_ORDER
                .iter()
                .all(|id| unique.contains(id))
        );
    }

    #[test]
    fn module_groups_have_stable_semantics_and_truthful_filtering() {
        assert!(DarkroomModuleGroup::Active.matches_title("anything"));
        assert!(DarkroomModuleGroup::Favorites.matches_title("Favorite presets"));
        assert!(DarkroomModuleGroup::Technical.matches_title("Lens correction"));
        assert!(DarkroomModuleGroup::Grading.matches_title("Color balance"));
        assert!(!DarkroomModuleGroup::Technical.matches_title("Exposure"));
    }
}
