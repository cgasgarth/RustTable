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
use crate::gui::darktable_components::{
    button as shared_button, dropdown as shared_dropdown, toggle_button as shared_toggle_button,
    toolbar as shared_toolbar,
};
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
    composition_guide: gtk4::DrawingArea,
    guides: gtk4::ToggleButton,
    left_panel: gtk4::ToggleButton,
    right_panel: gtk4::ToggleButton,
    filmstrip: gtk4::ToggleButton,
    canvas: Option<gtk4::Picture>,
    canvas_stage: Option<gtk4::Fixed>,
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

    pub(super) fn set_panel_visible(&self, panel: DarkroomPanelVisibility, visible: bool) {
        match panel {
            DarkroomPanelVisibility::Left => self.left_panel.set_active(visible),
            DarkroomPanelVisibility::Right => self.right_panel.set_active(visible),
            DarkroomPanelVisibility::Filmstrip => self.filmstrip.set_active(visible),
        }
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

    let guide_active = Rc::new(Cell::new(false));
    let composition_guide = composition_guide(preview, state, &guide_active);
    let guides = chrome_toggle(
        "darkroom-guides-toggle",
        "",
        "Show or hide rule-of-thirds composition guides",
    );
    guides.set_child(Some(&gtk4::Image::from_icon_name("view-grid-symbolic")));
    guides.set_active(false);
    guides.connect_toggled({
        let composition_guide = composition_guide.clone();
        let guide_active = Rc::clone(&guide_active);
        move |button| {
            guide_active.set(button.is_active());
            composition_guide.queue_draw();
        }
    });

    let zoom = shared_dropdown(
        DARKROOM_VIEWPORT_WIDGET_IDS[3],
        &DarkroomZoom::ALL.map(DarkroomZoom::label),
    );
    zoom.set_tooltip_text(Some("Image zoom level"));
    zoom.update_property(&[Property::Label("Image zoom level")]);
    let fit = chrome_button(
        DARKROOM_VIEWPORT_WIDGET_IDS[4],
        "fit",
        "Fit image to viewport",
    );
    let before_after = chrome_toggle(
        DARKROOM_VIEWPORT_WIDGET_IDS[5],
        "",
        "Compare before and after",
    );
    before_after.set_child(Some(&gtk4::Image::from_icon_name("view-dual-symbolic")));
    let soft_proof = chrome_toggle(DARKROOM_VIEWPORT_WIDGET_IDS[1], "", "Toggle soft proof");
    let soft_proof_glyph = gtk4::Label::new(Some("◐"));
    soft_proof_glyph.add_css_class("dt_symbolic_glyph");
    soft_proof.set_child(Some(&soft_proof_glyph));
    let gamut_check = chrome_toggle(DARKROOM_VIEWPORT_WIDGET_IDS[2], "", "Toggle gamut warning");
    gamut_check.set_child(Some(&gtk4::Image::from_icon_name("color-select-symbolic")));
    let top = toolbar("darkroom-toolbar-top", "Darkroom viewport controls");
    let left_panel = layout_toggle(
        "darkroom-left-panel-toggle",
        "sidebar-show-symbolic",
        "Show or hide the left darkroom panel",
        left_panel_visible.get(),
    );
    let right_panel = layout_toggle(
        "darkroom-right-panel-toggle",
        "sidebar-show-right-symbolic",
        "Show or hide the right darkroom panel",
        right_panel_visible.get(),
    );
    let filmstrip = layout_toggle(
        "darkroom-filmstrip-toggle",
        "view-list-symbolic",
        "Show or hide the bottom filmstrip",
        filmstrip_visible.get(),
    );
    let bottom = toolbar("darkroom-toolbar-bottom", "Darkroom layout controls");
    let utilities = toolbar("darkroom-utility-controls", "Darkroom display controls");
    top.set_visible(false);
    for control in [&left_panel, &right_panel, &filmstrip] {
        bottom.append(control);
    }
    bottom.append(&zoom);
    // "fit" already exists in Darktable's zoom selector. Keep the typed
    // button as an internal action target for compatibility, but do not add a
    // second visible control that duplicates the selector.
    fit.set_visible(false);
    for control in [&before_after, &soft_proof, &gamut_check, &guides] {
        utilities.append(control);
    }
    let (canvas, canvas_stage) = prepare_canvas_stage(preview);
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
        composition_guide: composition_guide.clone(),
        guides: guides.clone(),
        left_panel: left_panel.clone(),
        right_panel: right_panel.clone(),
        filmstrip: filmstrip.clone(),
        canvas,
        canvas_stage,
        sync_guard: Rc::new(Cell::new(false)),
    };

    let viewport = gtk4::Overlay::new();
    viewport.set_widget_name(DARKROOM_VIEWPORT_WIDGET_IDS[0]);
    viewport.set_hexpand(true);
    viewport.set_vexpand(true);
    viewport.set_size_request(-1, i32::from(DARKROOM_GEOMETRY.viewport_minimum_height_px));
    viewport.set_accessible_role(gtk4::AccessibleRole::Group);
    viewport.update_property(&[Property::Label("Darkroom image viewport")]);
    viewport.set_child(Some(preview.widget()));
    viewport.add_overlay(&composition_guide);
    viewport.add_overlay(&projection);
    viewport.add_overlay(&overlay);
    let boundary = gtk4::Separator::new(gtk4::Orientation::Horizontal);
    boundary.set_widget_name(DARKROOM_VIEWPORT_WIDGET_IDS[12]);
    boundary.update_property(&[Property::Label("Filmstrip boundary")]);

    page.append(&top);
    page.append(&viewport);
    let status_surface = DarkroomStatusSurface::new();
    status_surface.set_controls(&bottom);
    status_surface.set_utility_controls(&utilities);
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
    install_canvas_resize(&controls, preview, state);
    debug_assert_eq!(DARKROOM_VIEWPORT_FOCUS_ORDER.len(), 5);
    sync_viewport_controls(&controls, preview, state);
    (page, controls, status_surface)
}

fn composition_guide(
    preview: &PhotoPreview,
    state: &Rc<RefCell<DarkroomViewportState>>,
    active: &Rc<Cell<bool>>,
) -> gtk4::DrawingArea {
    let guide = gtk4::DrawingArea::new();
    guide.set_widget_name("darkroom-composition-guide");
    guide.set_hexpand(true);
    guide.set_vexpand(true);
    guide.set_halign(gtk4::Align::Fill);
    guide.set_valign(gtk4::Align::Fill);
    guide.set_can_target(false);
    guide.set_accessible_role(gtk4::AccessibleRole::Img);
    guide.update_property(&[Property::Label("Rule-of-thirds composition guide")]);
    let preview = preview.clone();
    let state = Rc::clone(state);
    let active = Rc::clone(active);
    guide.set_draw_func(move |_, context, width, height| {
        if !active.get() {
            return;
        }
        let Some(texture) = preview.texture() else {
            return;
        };
        draw_rule_of_thirds(
            context,
            width,
            height,
            texture.width(),
            texture.height(),
            *state.borrow(),
        );
    });
    guide
}

fn draw_rule_of_thirds(
    context: &gtk4::cairo::Context,
    viewport_width: i32,
    viewport_height: i32,
    image_width: i32,
    image_height: i32,
    state: DarkroomViewportState,
) {
    let Some(projection) = canvas_projection(
        viewport_width,
        viewport_height,
        image_width,
        image_height,
        i32::from(DARKROOM_GEOMETRY.image_border_px),
        state.zoom(),
        state.pan(),
    ) else {
        return;
    };
    let (left, top, image_width, image_height) = projection.image;
    let (clip_left, clip_top, clip_width, clip_height) = projection.viewport;
    let left = f64::from(left);
    let top = f64::from(top);
    let image_width = f64::from(image_width);
    let image_height = f64::from(image_height);
    let x_one = left + image_width / 3.0;
    let x_two = left + image_width * 2.0 / 3.0;
    let y_one = top + image_height / 3.0;
    let y_two = top + image_height * 2.0 / 3.0;

    context.save().expect("guide context can be saved");
    context.rectangle(
        f64::from(clip_left),
        f64::from(clip_top),
        f64::from(clip_width),
        f64::from(clip_height),
    );
    context.clip();
    context.set_dash(&[5.0], 0.0);
    for (width, red, green, blue, alpha) in
        [(2.0, 0.08, 0.08, 0.08, 0.80), (1.0, 0.92, 0.92, 0.92, 0.90)]
    {
        context.set_line_width(width);
        context.set_source_rgba(red, green, blue, alpha);
        for x in [x_one, x_two] {
            context.move_to(x, top);
            context.line_to(x, top + image_height);
        }
        for y in [y_one, y_two] {
            context.move_to(left, y);
            context.line_to(left + image_width, y);
        }
        let _ = context.stroke();
    }
    context.restore().expect("guide context can be restored");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CanvasProjection {
    viewport: (i32, i32, i32, i32),
    image: (i32, i32, i32, i32),
}

fn canvas_projection(
    viewport_width: i32,
    viewport_height: i32,
    image_width: i32,
    image_height: i32,
    border: i32,
    zoom: DarkroomZoom,
    pan: crate::viewport_presentation::ViewportPan,
) -> Option<CanvasProjection> {
    let inner_width = viewport_width.checked_sub(border.checked_mul(2)?)?;
    let inner_height = viewport_height.checked_sub(border.checked_mul(2)?)?;
    if inner_width <= 0 || inner_height <= 0 || image_width <= 0 || image_height <= 0 {
        return None;
    }
    let scale = projection_scale(inner_width, inner_height, image_width, image_height, zoom)?;
    let scaled_width = rounded_dimension(f64::from(image_width) * scale);
    let scaled_height = rounded_dimension(f64::from(image_height) * scale);
    let overflow_x = scaled_width.saturating_sub(inner_width);
    let overflow_y = scaled_height.saturating_sub(inner_height);
    let pan_x = i32::from(pan.x());
    let pan_y = i32::from(pan.y());
    let left = (viewport_width - scaled_width) / 2 - overflow_x * pan_x / 2_000;
    let top = (viewport_height - scaled_height) / 2 - overflow_y * pan_y / 2_000;
    Some(CanvasProjection {
        viewport: (border, border, inner_width, inner_height),
        image: (left, top, scaled_width, scaled_height),
    })
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn rounded_dimension(value: f64) -> i32 {
    value.round().clamp(1.0, f64::from(i32::MAX)) as i32
}

fn projection_scale(
    viewport_width: i32,
    viewport_height: i32,
    image_width: i32,
    image_height: i32,
    zoom: DarkroomZoom,
) -> Option<f64> {
    if viewport_width <= 0 || viewport_height <= 0 || image_width <= 0 || image_height <= 0 {
        return None;
    }
    let width_scale = f64::from(viewport_width) / f64::from(image_width);
    let height_scale = f64::from(viewport_height) / f64::from(image_height);
    Some(match zoom {
        DarkroomZoom::Small => width_scale.min(height_scale) * 0.5,
        DarkroomZoom::Fit => width_scale.min(height_scale),
        DarkroomZoom::Fill => width_scale.max(height_scale),
        _ => f64::from(zoom.percent()?) / 100.0,
    })
}

fn stepped_zoom(
    current: DarkroomZoom,
    zoom_in: bool,
    viewport: (i32, i32),
    image: (i32, i32),
    border: i32,
) -> Option<DarkroomZoom> {
    if !zoom_in {
        match current {
            DarkroomZoom::Small => return None,
            DarkroomZoom::Fit => return Some(DarkroomZoom::Small),
            DarkroomZoom::Fill => return Some(DarkroomZoom::Fit),
            _ => {}
        }
    } else if current == DarkroomZoom::Small {
        return Some(DarkroomZoom::Fit);
    }
    let inner_width = viewport.0.checked_sub(border.checked_mul(2)?)?;
    let inner_height = viewport.1.checked_sub(border.checked_mul(2)?)?;
    let current_scale = projection_scale(inner_width, inner_height, image.0, image.1, current)?;
    let mut best: Option<(DarkroomZoom, f64)> = None;
    for candidate in DarkroomZoom::ALL {
        if candidate == current {
            continue;
        }
        let Some(scale) = projection_scale(inner_width, inner_height, image.0, image.1, candidate)
        else {
            continue;
        };
        let eligible = if zoom_in {
            scale > current_scale + f64::EPSILON
        } else {
            scale < current_scale - f64::EPSILON
        };
        if !eligible {
            continue;
        }
        let improves = best.is_none_or(|(_, best_scale)| {
            if zoom_in {
                scale < best_scale
            } else {
                scale > best_scale
            }
        });
        if improves {
            best = Some((candidate, scale));
        }
    }
    best.map(|(zoom, _)| zoom)
}

fn layout_toggle(
    id: &str,
    icon_name: &str,
    accessible_name: &str,
    active: bool,
) -> gtk4::ToggleButton {
    let button = shared_toggle_button(id, "");
    button.set_child(Some(&gtk4::Image::from_icon_name(icon_name)));
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
    let toolbar = shared_toolbar(id, ThemeRole::Toolbar);
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
    let button = shared_toggle_button(id, label);
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
        if key == gdk::Key::g {
            controls_for_key
                .guides
                .set_active(!controls_for_key.guides.is_active());
            return gtk4::glib::Propagation::Stop;
        }
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
            gdk::Key::plus | gdk::Key::KP_Add => {
                viewport_zoom_action(true, &state_for_key, &controls_for_key, &preview_for_key)
            }
            gdk::Key::minus | gdk::Key::KP_Subtract => {
                viewport_zoom_action(false, &state_for_key, &controls_for_key, &preview_for_key)
            }
            gdk::Key::_0 | gdk::Key::KP_0 => Some(DarkroomViewportAction::Fit),
            gdk::Key::b => Some(DarkroomViewportAction::ToggleBeforeAfter),
            gdk::Key::s => Some(DarkroomViewportAction::ToggleSoftProof),
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
    let scroll = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
    let state_for_scroll = Rc::clone(state);
    let handler_for_scroll = Rc::clone(handler);
    let controls_for_scroll = controls.clone();
    let preview_for_scroll = preview.clone();
    scroll.connect_scroll(move |_, _, delta_y| {
        let Some(zoom_in) = scroll_zoom_direction(delta_y) else {
            return gtk4::glib::Propagation::Proceed;
        };
        let Some(action) = viewport_zoom_action(
            zoom_in,
            &state_for_scroll,
            &controls_for_scroll,
            &preview_for_scroll,
        ) else {
            return gtk4::glib::Propagation::Proceed;
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
        // Darktable moves the viewport center opposite the pointer delta, so
        // the zoomed image itself follows the drag.
        let delta_x = rounded_delta(previous.0 - offset_x);
        let delta_y = rounded_delta(previous.1 - offset_y);
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

fn scroll_zoom_direction(delta_y: f64) -> Option<bool> {
    if !delta_y.is_finite() || delta_y == 0.0 {
        return None;
    }
    Some(delta_y.is_sign_negative())
}

fn viewport_zoom_action(
    zoom_in: bool,
    state: &Rc<RefCell<DarkroomViewportState>>,
    controls: &ViewportControls,
    preview: &PhotoPreview,
) -> Option<DarkroomViewportAction> {
    let projection_container = controls.canvas_stage.as_ref()?;
    let texture = preview.texture()?;
    stepped_zoom(
        state.borrow().zoom(),
        zoom_in,
        (projection_container.width(), projection_container.height()),
        (texture.width(), texture.height()),
        0,
    )
    .map(DarkroomViewportAction::SetZoom)
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
    let overlay_active = state.has_active_overlay();
    let projection = overlay_active.then(|| state.projection_label());
    controls
        .projection
        .set_text(projection.as_deref().unwrap_or_default());
    controls.projection.set_visible(overlay_active);
    controls
        .overlay_before
        .set_visible(state.comparison() == ViewportComparison::Before);
    controls
        .overlay_soft_proof
        .set_visible(state.color_mode() == ViewportColorMode::SoftProof);
    controls
        .overlay_gamut
        .set_visible(state.color_mode() == ViewportColorMode::GamutCheck);
    apply_canvas_projection(
        controls.canvas.as_ref(),
        controls.canvas_stage.as_ref(),
        preview,
        state,
    );
    controls.composition_guide.queue_draw();
    controls.sync_guard.set(false);
}

fn apply_canvas_projection(
    canvas: Option<&gtk4::Picture>,
    container: Option<&gtk4::Fixed>,
    preview: &PhotoPreview,
    viewport_state: DarkroomViewportState,
) {
    let (Some(canvas), Some(container), Some(texture)) = (canvas, container, preview.texture())
    else {
        return;
    };
    let Some(projection) = canvas_projection(
        container.width(),
        container.height(),
        texture.width(),
        texture.height(),
        0,
        viewport_state.zoom(),
        viewport_state.pan(),
    ) else {
        return;
    };
    let (left, top, width, height) = projection.image;
    // The fixed stage owns the requested projection size. Keeping Picture
    // shrinkable lets GTK honor fit/small requests below the texture's natural
    // pixel dimensions instead of clipping a 1:1 natural-size allocation.
    canvas.set_can_shrink(true);
    canvas.set_size_request(width, height);
    container.move_(canvas, f64::from(left), f64::from(top));
}

fn install_canvas_resize(
    controls: &ViewportControls,
    preview: &PhotoPreview,
    state: &Rc<RefCell<DarkroomViewportState>>,
) {
    let (Some(canvas), Some(container)) = (controls.canvas.clone(), controls.canvas_stage.clone())
    else {
        return;
    };
    let preview = preview.clone();
    let state = Rc::clone(state);
    controls.composition_guide.connect_resize(move |_, _, _| {
        apply_canvas_projection(Some(&canvas), Some(&container), &preview, *state.borrow());
    });
}

fn prepare_canvas_stage(preview: &PhotoPreview) -> (Option<gtk4::Picture>, Option<gtk4::Fixed>) {
    let Some(canvas) = find_image_canvas(preview.widget().upcast_ref()) else {
        return (None, None);
    };
    let Some(parent) = canvas
        .parent()
        .and_then(|parent| parent.downcast::<gtk4::Overlay>().ok())
    else {
        return (Some(canvas), None);
    };
    parent.set_child(None::<&gtk4::Widget>);
    let stage = gtk4::Fixed::new();
    stage.set_widget_name("darkroom-image-stage");
    stage.set_hexpand(true);
    stage.set_vexpand(true);
    stage.set_overflow(gtk4::Overflow::Hidden);
    parent.set_child(Some(&stage));
    canvas.set_hexpand(false);
    canvas.set_vexpand(false);
    canvas.set_halign(gtk4::Align::Start);
    canvas.set_valign(gtk4::Align::Start);
    stage.put(&canvas, 0.0, 0.0);
    (Some(canvas), Some(stage))
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
    let button = shared_button(id, label);
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

#[cfg(test)]
#[path = "viewport_tests.rs"]
mod tests;
