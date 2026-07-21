//! Darkroom viewport construction and typed input routing.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::accessible::Property;
use gtk4::gdk;
use gtk4::prelude::*;

use super::status::DarkroomStatusSurface;
use super::{
    DARKROOM_GEOMETRY, DARKROOM_VIEWPORT_FOCUS_ORDER, DARKROOM_VIEWPORT_WIDGET_IDS,
    DarkroomPanelVisibility, DarkroomPanelVisibilityAction, DarkroomPanelVisibilityHandler,
    DarkroomViewportActionHandler, PhotoPreview, ThemeRole, apply_theme_role,
};
use crate::HistogramSample;
use crate::viewport_presentation::{
    DarkroomViewportAction, DarkroomViewportCommand, DarkroomViewportState, DarkroomZoom,
    ViewportColorMode, ViewportComparison,
};

#[derive(Clone)]
pub(super) struct ViewportControls {
    zoom: gtk4::DropDown,
    fit: gtk4::Button,
    before_after: gtk4::ToggleButton,
    soft_proof: gtk4::ToggleButton,
    gamut_check: gtk4::ToggleButton,
    projection: gtk4::Label,
    overlay_before: gtk4::Label,
    overlay_soft_proof: gtk4::Label,
    overlay_gamut: gtk4::Label,
    overlay_sample: gtk4::Label,
    canvas: Option<gtk4::Picture>,
    sync_guard: Rc<Cell<bool>>,
}

impl ViewportControls {
    pub(super) fn set_histogram_sample(&self, sample: HistogramSample) {
        let values = sample.values();
        self.overlay_sample.set_text(&format!(
            "histogram sample · bin {} · R {} · G {} · B {} · L {}",
            sample.bin(),
            values.red(),
            values.green(),
            values.blue(),
            values.luminance()
        ));
        self.overlay_sample.set_visible(true);
    }

    pub(super) fn clear_histogram_sample(&self) {
        self.overlay_sample.set_text("");
        self.overlay_sample.set_visible(false);
    }
}

#[allow(clippy::too_many_lines)]
pub(super) fn darkroom_page(
    preview: &PhotoPreview,
    state: &Rc<RefCell<DarkroomViewportState>>,
    handler: &Rc<RefCell<Option<DarkroomViewportActionHandler>>>,
    left_panel_visible: &Rc<Cell<bool>>,
    right_panel_visible: &Rc<Cell<bool>>,
    filmstrip_visible: &Rc<Cell<bool>>,
    panel_visibility_handler: &Rc<RefCell<Option<DarkroomPanelVisibilityHandler>>>,
) -> (gtk4::Box, ViewportControls, DarkroomStatusSurface) {
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

    let overlay_before = viewport_badge(
        "darkroom-overlay-before",
        "before preview unavailable",
        "Before/after viewport state",
    );
    let overlay_soft_proof = viewport_badge(
        "darkroom-overlay-soft-proof",
        "soft proof requested · transform unavailable",
        "Soft-proof viewport state",
    );
    let overlay_gamut = viewport_badge(
        "darkroom-overlay-gamut",
        "gamut warning requested · analysis unavailable",
        "Gamut-warning viewport state",
    );
    let overlay_sample = viewport_badge(
        "darkroom-overlay-histogram-sample",
        "",
        "Selected histogram sample",
    );
    let overlay = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    overlay.set_widget_name("darkroom-viewport-overlay");
    overlay.set_halign(gtk4::Align::Start);
    overlay.set_valign(gtk4::Align::Start);
    overlay.set_margin_start(10);
    overlay.set_margin_top(10);
    overlay.append(&overlay_before);
    overlay.append(&overlay_soft_proof);
    overlay.append(&overlay_gamut);
    overlay.append(&overlay_sample);

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
        overlay_before: overlay_before.clone(),
        overlay_soft_proof: overlay_soft_proof.clone(),
        overlay_gamut: overlay_gamut.clone(),
        overlay_sample: overlay_sample.clone(),
        canvas: find_image_canvas(preview.widget().upcast_ref()),
        sync_guard: Rc::new(Cell::new(false)),
    };

    let top = toolbar("darkroom-toolbar-top", "Darkroom color proofing controls");
    let left_panel = layout_toggle(
        "darkroom-left-panel-toggle",
        "left panel",
        "Show or hide the left darkroom panel",
        left_panel_visible.get(),
    );
    let right_panel = layout_toggle(
        "darkroom-right-panel-toggle",
        "right panel",
        "Show or hide the right darkroom panel",
        right_panel_visible.get(),
    );
    let filmstrip = layout_toggle(
        "darkroom-filmstrip-toggle",
        "filmstrip",
        "Show or hide the bottom filmstrip",
        filmstrip_visible.get(),
    );
    top.append(&left_panel);
    top.append(&right_panel);
    top.append(&filmstrip);
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
    viewport.set_size_request(-1, i32::from(DARKROOM_GEOMETRY.viewport_minimum_height_px));
    viewport.set_accessible_role(gtk4::AccessibleRole::Group);
    viewport.update_property(&[Property::Label("Darkroom image viewport")]);
    viewport.set_child(Some(preview.widget()));
    viewport.add_overlay(&projection);
    viewport.add_overlay(&overlay);
    let boundary = gtk4::Separator::new(gtk4::Orientation::Horizontal);
    boundary.set_widget_name(DARKROOM_VIEWPORT_WIDGET_IDS[7]);
    boundary.update_property(&[Property::Label("Filmstrip boundary")]);

    page.append(&top);
    page.append(&viewport);
    page.append(&bottom);
    let status_surface = DarkroomStatusSurface::new();
    page.append(status_surface.widget());
    page.append(&boundary);
    connect_viewport_controls(&controls, state, handler, preview);
    connect_layout_toggle(
        &left_panel,
        left_panel_visible,
        panel_visibility_handler,
        DarkroomPanelVisibility::Left,
    );
    connect_layout_toggle(
        &right_panel,
        right_panel_visible,
        panel_visibility_handler,
        DarkroomPanelVisibility::Right,
    );
    connect_layout_toggle(
        &filmstrip,
        filmstrip_visible,
        panel_visibility_handler,
        DarkroomPanelVisibility::Filmstrip,
    );
    install_viewport_input(&page, preview, &controls, state, handler);
    debug_assert_eq!(DARKROOM_VIEWPORT_FOCUS_ORDER.len(), 5);
    sync_viewport_controls(&controls, preview, state);
    (page, controls, status_surface)
}

fn layout_toggle(id: &str, label: &str, accessible_name: &str, active: bool) -> gtk4::ToggleButton {
    let button = gtk4::ToggleButton::with_label(label);
    button.set_widget_name(id);
    button.set_active(active);
    button.add_css_class("dt_button");
    button.set_focus_on_click(false);
    button.set_tooltip_text(Some(accessible_name));
    button.update_property(&[Property::Label(accessible_name)]);
    button
}

fn connect_layout_toggle(
    button: &gtk4::ToggleButton,
    state: &Rc<Cell<bool>>,
    handler: &Rc<RefCell<Option<DarkroomPanelVisibilityHandler>>>,
    panel: DarkroomPanelVisibility,
) {
    let state = Rc::clone(state);
    let handler = Rc::clone(handler);
    button.connect_toggled(move |button| {
        let visible = button.is_active();
        state.set(visible);
        if let Some(handler) = handler.borrow().as_ref() {
            handler(DarkroomPanelVisibilityAction::new(panel, visible));
        }
    });
}

fn toolbar(id: &str, accessible_name: &str) -> gtk4::Box {
    let toolbar = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);
    toolbar.set_widget_name(id);
    toolbar.add_css_class("dt_darkroom_toolbar");
    toolbar.set_accessible_role(gtk4::AccessibleRole::Toolbar);
    toolbar.update_property(&[Property::Label(accessible_name)]);
    toolbar.set_height_request(match id {
        "darkroom-toolbar-top" => i32::from(DARKROOM_GEOMETRY.top_toolbar_height_px),
        _ => i32::from(DARKROOM_GEOMETRY.bottom_toolbar_height_px),
    });
    toolbar
}

pub(crate) fn chrome_toggle(id: &str, label: &str, accessible_name: &str) -> gtk4::ToggleButton {
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
    if controls.sync_guard.get() || state.borrow().photo_id().is_none() {
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

pub(super) fn sync_viewport_controls(
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
    controls
        .overlay_before
        .set_visible(state.comparison() == ViewportComparison::Before);
    controls
        .overlay_soft_proof
        .set_visible(state.color_mode() == ViewportColorMode::SoftProof);
    controls
        .overlay_gamut
        .set_visible(state.color_mode() == ViewportColorMode::GamutCheck);
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

fn chrome_button(id: &str, label: &str, accessible_name: &str) -> gtk4::Button {
    let button = gtk4::Button::with_label(label);
    button.set_widget_name(id);
    button.add_css_class("dt_button");
    button.set_focus_on_click(false);
    button.update_property(&[Property::Label(accessible_name)]);
    button
}

fn viewport_badge(id: &str, text: &str, accessible_name: &str) -> gtk4::Label {
    let badge = gtk4::Label::new(Some(text));
    badge.set_widget_name(id);
    badge.set_halign(gtk4::Align::Start);
    badge.add_css_class("darkroom_viewport_badge");
    badge.add_css_class("warning");
    badge.set_tooltip_text(Some(accessible_name));
    badge.set_accessible_role(gtk4::AccessibleRole::Status);
    badge.update_property(&[Property::Label(accessible_name)]);
    badge.set_visible(false);
    badge
}
