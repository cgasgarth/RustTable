//! Source-mapped `GTK4` presentation for Darktable's Color Correction v1 module.
//!
//! The parameter order, grid geometry, endpoint selection rules, pointer and
//! keyboard interactions, presets, and labels come from
//! `src/iop/colorcorrection.c`.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk4::{gdk, glib, prelude::*};

use crate::{
    bauhaus::slider_popup::{reset_source_scroll_units, source_scroll_unit_delta},
    presentation::SourceMappedSliderSpec,
};

pub const COLORCORRECTION_MODULE_ID: &str = "colorcorrection";
pub const COLORCORRECTION_GRID_TOOLTIP: &str = "drag the line for split-toning. bright means highlights, dark means shadows. use mouse wheel to change saturation.";
pub const COLORCORRECTION_GRID_CELLS: usize = 8;
pub const COLORCORRECTION_GRID_INSET: f64 = 5.0;
pub const COLORCORRECTION_GRID_MAX: f64 = 40.0;
pub const COLORCORRECTION_KEY_STEP: f64 = 0.5;

/// Native module metadata and its only ordinary Bauhaus parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorCorrectionSourceMap {
    title: &'static str,
    group_keys: &'static [&'static str],
    default_enabled: bool,
    default_expanded: bool,
}

impl ColorCorrectionSourceMap {
    #[must_use]
    pub const fn title(self) -> &'static str {
        self.title
    }

    #[must_use]
    pub const fn group_keys(self) -> &'static [&'static str] {
        self.group_keys
    }

    #[must_use]
    pub const fn default_enabled(self) -> bool {
        self.default_enabled
    }

    #[must_use]
    pub const fn default_expanded(self) -> bool {
        self.default_expanded
    }

    #[must_use]
    pub const fn saturation(self, parameter_id: &str) -> Option<ColorCorrectionSliderSourceMap> {
        if const_str_equal(parameter_id, COLORCORRECTION_SATURATION.parameter_id) {
            Some(COLORCORRECTION_SATURATION)
        } else {
            None
        }
    }
}

const fn const_str_equal(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len() != right.len() {
        return false;
    }
    let mut index = 0;
    while index < left.len() {
        if left[index] != right[index] {
            return false;
        }
        index += 1;
    }
    true
}

/// Source metadata for the native saturation slider.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorCorrectionSliderSourceMap {
    parameter_id: &'static str,
    label: &'static str,
    minimum: f64,
    maximum: f64,
    default_value: f64,
    digits: i32,
    step: f64,
    automatic_step: bool,
    tooltip: &'static str,
}

impl ColorCorrectionSliderSourceMap {
    #[must_use]
    pub const fn parameter_id(self) -> &'static str {
        self.parameter_id
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        self.label
    }

    #[must_use]
    pub const fn range(self) -> (f64, f64) {
        (self.minimum, self.maximum)
    }

    #[must_use]
    pub const fn default_value(self) -> f64 {
        self.default_value
    }

    #[must_use]
    pub const fn digits(self) -> i32 {
        self.digits
    }

    #[must_use]
    pub const fn step(self) -> f64 {
        self.step
    }

    #[must_use]
    pub const fn automatic_step(self) -> bool {
        self.automatic_step
    }

    #[must_use]
    pub const fn tooltip(self) -> &'static str {
        self.tooltip
    }

    pub(crate) const fn slider_presentation(self) -> SourceMappedSliderSpec {
        SourceMappedSliderSpec::new("", self.digits, self.automatic_step, self.tooltip)
    }
}

pub const COLORCORRECTION_SOURCE_MAP: ColorCorrectionSourceMap = ColorCorrectionSourceMap {
    title: "color correction",
    group_keys: &["group.color", "group.grading"],
    default_enabled: false,
    default_expanded: false,
};

pub const COLORCORRECTION_SATURATION: ColorCorrectionSliderSourceMap =
    ColorCorrectionSliderSourceMap {
        parameter_id: "saturation",
        label: "saturation",
        minimum: -3.0,
        maximum: 3.0,
        default_value: 1.0,
        digits: 2,
        step: 0.01,
        automatic_step: true,
        tooltip: "set the global saturation",
    };

/// The four endpoint parameters manipulated together by the native grid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorCorrectionGridState {
    hia: f64,
    hib: f64,
    loa: f64,
    lob: f64,
}

impl ColorCorrectionGridState {
    pub const DEFAULT: Self = Self {
        hia: 0.0,
        hib: 0.0,
        loa: 0.0,
        lob: 0.0,
    };

    /// Constructs a finite persisted grid state without clamping imported data.
    ///
    /// # Errors
    ///
    /// Returns the first non-finite native parameter name.
    pub fn new(
        hia: f64,
        hib: f64,
        loa: f64,
        lob: f64,
    ) -> Result<Self, ColorCorrectionGridStateError> {
        for (parameter, value) in [("hia", hia), ("hib", hib), ("loa", loa), ("lob", lob)] {
            if !value.is_finite() {
                return Err(ColorCorrectionGridStateError { parameter });
            }
        }
        Ok(Self { hia, hib, loa, lob })
    }

    #[must_use]
    pub const fn hia(self) -> f64 {
        self.hia
    }

    #[must_use]
    pub const fn hib(self) -> f64 {
        self.hib
    }

    #[must_use]
    pub const fn loa(self) -> f64 {
        self.loa
    }

    #[must_use]
    pub const fn lob(self) -> f64 {
        self.lob
    }

    #[must_use]
    pub const fn endpoint(self, endpoint: ColorCorrectionEndpoint) -> (f64, f64) {
        match endpoint {
            ColorCorrectionEndpoint::Shadows => (self.loa, self.lob),
            ColorCorrectionEndpoint::Highlights => (self.hia, self.hib),
        }
    }

    #[must_use]
    const fn with_endpoint(self, endpoint: ColorCorrectionEndpoint, a: f64, b: f64) -> Self {
        match endpoint {
            ColorCorrectionEndpoint::Shadows => Self {
                loa: a,
                lob: b,
                ..self
            },
            ColorCorrectionEndpoint::Highlights => Self {
                hia: a,
                hib: b,
                ..self
            },
        }
    }

    #[must_use]
    const fn reset_endpoint(self, endpoint: ColorCorrectionEndpoint) -> Self {
        self.with_endpoint(endpoint, 0.0, 0.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorCorrectionGridStateError {
    parameter: &'static str,
}

impl ColorCorrectionGridStateError {
    #[must_use]
    pub const fn parameter(self) -> &'static str {
        self.parameter
    }
}

impl std::fmt::Display for ColorCorrectionGridStateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Color Correction parameter {} must be finite",
            self.parameter
        )
    }
}

impl std::error::Error for ColorCorrectionGridStateError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorCorrectionEndpoint {
    Shadows,
    Highlights,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorCorrectionDoubleClick {
    Endpoint(ColorCorrectionGridState),
    AllDefaults,
}

/// Pure interaction state shared by `GTK` callbacks and source-derived tests.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorCorrectionGridModel {
    state: ColorCorrectionGridState,
    selected: Option<ColorCorrectionEndpoint>,
}

impl ColorCorrectionGridModel {
    #[must_use]
    pub const fn new(state: ColorCorrectionGridState) -> Self {
        Self {
            state,
            selected: None,
        }
    }

    #[must_use]
    pub const fn state(self) -> ColorCorrectionGridState {
        self.state
    }

    #[must_use]
    pub const fn selected(self) -> Option<ColorCorrectionEndpoint> {
        self.selected
    }

    /// Updates the source hover selection. Equal-distance ties select highlights.
    pub fn hover(&mut self, x: f64, y: f64, width: f64, height: f64) {
        let Some((mouse_a, mouse_b)) = pointer_to_lab(x, y, width, height) else {
            self.selected = None;
            return;
        };
        let (loa, lob) = self.state.endpoint(ColorCorrectionEndpoint::Shadows);
        let (hia, hib) = self.state.endpoint(ColorCorrectionEndpoint::Highlights);
        let shadow_distance = (loa - mouse_a).mul_add(loa - mouse_a, (lob - mouse_b).powi(2));
        let highlight_distance = (hia - mouse_a).mul_add(hia - mouse_a, (hib - mouse_b).powi(2));
        let threshold_squared = COLORCORRECTION_GRID_INSET.powi(2);
        self.selected = if shadow_distance < threshold_squared
            && shadow_distance < highlight_distance
        {
            Some(ColorCorrectionEndpoint::Shadows)
        } else if highlight_distance < threshold_squared && highlight_distance <= shadow_distance {
            Some(ColorCorrectionEndpoint::Highlights)
        } else {
            None
        };
    }

    /// Moves only the selected endpoint to the clamped pointer coordinate.
    #[must_use]
    pub fn drag(
        &mut self,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> Option<ColorCorrectionGridState> {
        let endpoint = self.selected?;
        let (a, b) = pointer_to_lab(x, y, width, height)?;
        self.state = self.state.with_endpoint(endpoint, a, b);
        Some(self.state)
    }

    #[must_use]
    pub const fn double_click(&mut self) -> ColorCorrectionDoubleClick {
        let Some(endpoint) = self.selected else {
            self.state = ColorCorrectionGridState::DEFAULT;
            return ColorCorrectionDoubleClick::AllDefaults;
        };
        self.state = self.state.reset_endpoint(endpoint);
        ColorCorrectionDoubleClick::Endpoint(self.state)
    }

    /// Applies the native 0.5-unit arrow step and caller-supplied speed.
    #[must_use]
    pub fn nudge(
        &mut self,
        delta_a: f64,
        delta_b: f64,
        speed: f64,
    ) -> Option<ColorCorrectionGridState> {
        let endpoint = self.selected?;
        let (a, b) = self.state.endpoint(endpoint);
        let a = (delta_a * COLORCORRECTION_KEY_STEP)
            .mul_add(speed, a)
            .clamp(-COLORCORRECTION_GRID_MAX, COLORCORRECTION_GRID_MAX);
        let b = (delta_b * COLORCORRECTION_KEY_STEP)
            .mul_add(speed, b)
            .clamp(-COLORCORRECTION_GRID_MAX, COLORCORRECTION_GRID_MAX);
        self.state = self.state.with_endpoint(endpoint, a, b);
        Some(self.state)
    }
}

#[must_use]
pub fn scrolled_saturation(current: f64, unit_delta: i32) -> f64 {
    0.1f64.mul_add(-f64::from(unit_delta), current).clamp(
        COLORCORRECTION_SATURATION.minimum,
        COLORCORRECTION_SATURATION.maximum,
    )
}

fn pointer_to_lab(x: f64, y: f64, width: f64, height: f64) -> Option<(f64, f64)> {
    let inner_width = 2.0f64.mul_add(-COLORCORRECTION_GRID_INSET, width);
    let inner_height = 2.0f64.mul_add(-COLORCORRECTION_GRID_INSET, height);
    if !x.is_finite()
        || !y.is_finite()
        || !inner_width.is_finite()
        || !inner_height.is_finite()
        || inner_width <= 0.0
        || inner_height <= 0.0
    {
        return None;
    }
    let mouse_x = (x - COLORCORRECTION_GRID_INSET).clamp(0.0, inner_width);
    let mouse_y = (inner_height - 1.0 - y + COLORCORRECTION_GRID_INSET).clamp(0.0, inner_height);
    let a = 2.0f64.mul_add(mouse_x, -inner_width) * COLORCORRECTION_GRID_MAX / inner_width;
    let b = 2.0f64.mul_add(mouse_y, -inner_height) * COLORCORRECTION_GRID_MAX / inner_height;
    Some((a, b))
}

pub(crate) type ColorCorrectionGridCommit = Rc<dyn Fn(ColorCorrectionGridState) -> bool>;
pub(crate) type ColorCorrectionResetCommit = Rc<dyn Fn() -> bool>;

pub(crate) struct ColorCorrectionGridGtkContext {
    pub(crate) widget_id: String,
    pub(crate) state: ColorCorrectionGridState,
    pub(crate) saturation: gtk4::Scale,
    /// Raw persisted saturation used by the native grid before any UI edit.
    ///
    /// Darktable draws imported finite outliers verbatim even though its
    /// Bauhaus slider displays a hard-range-clamped value.
    pub(crate) initial_saturation: f64,
    pub(crate) sensitive: bool,
    pub(crate) commit_grid: Option<ColorCorrectionGridCommit>,
    pub(crate) reset_all: Option<ColorCorrectionResetCommit>,
}

/// Builds the native square grid using `GTK4` event controllers.
///
/// `GTK4` replaces `GTK3` event masks with controllers. Smooth scroll units use
/// the GTK-main-thread source helper shared by every migrated caller, while
/// shortcut speed uses Darktable's default modifier mapping because `RustTable`
/// does not yet expose Darktable's user-configured accelerator speed table.
/// The native `dt_gui_ignore_scroll` sidebar preference and its `Ctrl+Alt`
/// override have no existing Rust UI policy seam, so this milestone retains
/// the controller's direct vertical-scroll handling. `DT_PIXEL_APPLY_DPI`
/// sizing/hover scaling likewise remains unqualified, and `GTK4`'s resize
/// request below substitutes for Darktable's `GTK3` height-for-width helper.
///
/// Native adds a mergeable history item on every drag motion. `RustTable`'s
/// current application bridge reprojects the complete module rail after each
/// persisted edit, which would destroy this controller after the first motion.
/// Grid motion is therefore kept live locally and committed once on release;
/// this is the bounded history-coalescing residual until the rail owns a
/// stack-wide live snapshot/revision reconciliation seam.
#[expect(
    clippy::too_many_lines,
    reason = "The source Color Correction grid keeps draw geometry and GTK pointer/key callbacks together."
)]
pub(crate) fn build_grid(context: ColorCorrectionGridGtkContext) -> gtk4::DrawingArea {
    let ColorCorrectionGridGtkContext {
        widget_id,
        state,
        saturation,
        initial_saturation,
        sensitive,
        commit_grid,
        reset_all,
    } = context;
    let area = gtk4::DrawingArea::new();
    area.set_widget_name(&format!("{widget_id}-grid"));
    area.set_hexpand(true);
    area.set_halign(gtk4::Align::Fill);
    area.set_height_request(1);
    area.set_focusable(true);
    area.set_sensitive(sensitive);
    area.set_tooltip_text(Some(COLORCORRECTION_GRID_TOOLTIP));
    area.set_accessible_role(gtk4::AccessibleRole::Slider);
    area.update_property(&[gtk4::accessible::Property::Label(
        "Color Correction split-toning grid",
    )]);
    area.connect_resize(|area, width, _| {
        if width > 0 && area.height_request() != width {
            area.set_height_request(width);
        }
        area.queue_draw();
    });

    let model = Rc::new(RefCell::new(ColorCorrectionGridModel::new(state)));
    // Preserve raw imported outliers for the first draw. The first actual
    // slider/wheel edit intentionally switches to the bounded GTK value.
    let saturation_value = Rc::new(Cell::new(initial_saturation));
    let saturation_for_draw = Rc::clone(&saturation_value);
    let saturation_for_scroll = Rc::clone(&saturation_value);
    let area_for_saturation = area.downgrade();
    saturation.connect_value_changed(move |scale| {
        saturation_for_draw.set(scale.value());
        if let Some(area) = area_for_saturation.upgrade() {
            area.queue_draw();
        }
    });

    let model_for_draw = Rc::clone(&model);
    area.set_draw_func(move |_, cairo, width, height| {
        let model = model_for_draw.borrow();
        draw_grid(
            cairo,
            width,
            height,
            model.state(),
            model.selected(),
            saturation_value.get(),
        );
    });

    let primary_down = Rc::new(Cell::new(false));
    let drag_origin = Rc::new(Cell::new(None::<ColorCorrectionGridState>));
    let motion = gtk4::EventControllerMotion::new();
    motion.set_name(Some("dt-colorcorrection-motion"));
    {
        let area = area.downgrade();
        let model = Rc::clone(&model);
        let primary_down = Rc::clone(&primary_down);
        motion.connect_motion(move |_, x, y| {
            let Some(area) = area.upgrade() else {
                return;
            };
            if primary_down.get() {
                let _ = model.borrow_mut().drag(
                    x,
                    y,
                    f64::from(area.allocated_width()),
                    f64::from(area.allocated_height()),
                );
            } else {
                model.borrow_mut().hover(
                    x,
                    y,
                    f64::from(area.allocated_width()),
                    f64::from(area.allocated_height()),
                );
            }
            if model.borrow().selected().is_some() {
                area.grab_focus();
            }
            area.queue_draw();
        });
    }
    {
        let area = area.downgrade();
        motion.connect_leave(move |_| {
            if let Some(area) = area.upgrade() {
                area.queue_draw();
            }
        });
    }
    area.add_controller(motion);

    let click = gtk4::GestureClick::new();
    click.set_name(Some("dt-colorcorrection-click"));
    click.set_button(1);
    click.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let area = area.downgrade();
        let model = Rc::clone(&model);
        let primary_down = Rc::clone(&primary_down);
        let drag_origin = Rc::clone(&drag_origin);
        let commit_grid = commit_grid.clone();
        click.connect_pressed(move |gesture, press_count, _, _| {
            let Some(area) = area.upgrade() else {
                return;
            };
            primary_down.set(true);
            if press_count != 2 {
                drag_origin.set(Some(model.borrow().state()));
                return;
            }
            drag_origin.set(None);
            let previous = model.borrow().state();
            let accepted = match model.borrow_mut().double_click() {
                ColorCorrectionDoubleClick::Endpoint(next) => {
                    commit_grid.as_ref().is_none_or(|commit| commit(next))
                }
                ColorCorrectionDoubleClick::AllDefaults => {
                    reset_all.as_ref().is_none_or(|reset| reset())
                }
            };
            if !accepted {
                *model.borrow_mut() = ColorCorrectionGridModel::new(previous);
            }
            primary_down.set(false);
            area.queue_draw();
            let _ = gesture.set_state(gtk4::EventSequenceState::Claimed);
        });
    }
    {
        let area = area.downgrade();
        let model = Rc::clone(&model);
        let primary_down = Rc::clone(&primary_down);
        let drag_origin = Rc::clone(&drag_origin);
        let commit_grid = commit_grid.clone();
        click.connect_released(move |_, _, _, _| {
            primary_down.set(false);
            let Some(previous) = drag_origin.take() else {
                return;
            };
            let next = model.borrow().state();
            if next != previous && !commit_grid.as_ref().is_none_or(|commit| commit(next)) {
                *model.borrow_mut() = ColorCorrectionGridModel::new(previous);
            }
            if let Some(area) = area.upgrade() {
                area.queue_draw();
            }
        });
    }
    {
        let area = area.downgrade();
        let model = Rc::clone(&model);
        let primary_down = Rc::clone(&primary_down);
        let drag_origin = Rc::clone(&drag_origin);
        click.connect_cancel(move |_, _| {
            primary_down.set(false);
            if let Some(previous) = drag_origin.take() {
                *model.borrow_mut() = ColorCorrectionGridModel::new(previous);
            }
            if let Some(area) = area.upgrade() {
                area.queue_draw();
            }
        });
    }
    area.add_controller(click);

    let scroll = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::BOTH_AXES);
    scroll.set_name(Some("dt-colorcorrection-scroll"));
    scroll.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let saturation = saturation.downgrade();
        let saturation_value = Rc::clone(&saturation_for_scroll);
        scroll.connect_scroll(move |controller, delta_x, delta_y| {
            if controller
                .current_event()
                .is_some_and(|event| event.is_pointer_emulated())
            {
                return glib::Propagation::Stop;
            }
            let delta = colorcorrection_scroll_unit_delta(controller.unit(), delta_x, delta_y);
            if let (Some(delta), Some(saturation)) = (delta, saturation.upgrade()) {
                let previous_display = saturation.value();
                saturation.set_value(scrolled_saturation(saturation_value.get(), delta));
                if saturation.value().to_bits() == previous_display.to_bits() {
                    // An imported native value can lie outside the Bauhaus
                    // hard range while GTK displays the clamped endpoint.
                    // The first wheel edit still has to persist that endpoint.
                    saturation.emit_by_name::<()>("value-changed", &[]);
                }
            }
            glib::Propagation::Stop
        });
    }
    scroll.connect_scroll_end(|controller| {
        if !controller
            .current_event()
            .is_some_and(|event| event.is_pointer_emulated())
        {
            colorcorrection_scroll_end();
        }
    });
    area.add_controller(scroll);

    let key = gtk4::EventControllerKey::new();
    key.set_name(Some("dt-colorcorrection-key"));
    key.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let area = area.downgrade();
        let model = Rc::clone(&model);
        key.connect_key_pressed(move |_, key, _, modifiers| {
            let (delta_a, delta_b) = match key {
                gdk::Key::Up | gdk::Key::KP_Up => (0.0, 1.0),
                gdk::Key::Down | gdk::Key::KP_Down => (0.0, -1.0),
                gdk::Key::Right | gdk::Key::KP_Right => (1.0, 0.0),
                gdk::Key::Left | gdk::Key::KP_Left => (-1.0, 0.0),
                _ => return glib::Propagation::Proceed,
            };
            let previous = model.borrow().state();
            let Some(next) = model
                .borrow_mut()
                .nudge(delta_a, delta_b, modifier_speed(modifiers))
            else {
                return glib::Propagation::Proceed;
            };
            if !commit_grid.as_ref().is_none_or(|commit| commit(next)) {
                *model.borrow_mut() = ColorCorrectionGridModel::new(previous);
            }
            if let Some(area) = area.upgrade() {
                area.queue_draw();
            }
            glib::Propagation::Stop
        });
    }
    area.add_controller(key);
    area
}

/// Normalizes both grid scroll axes through the shared source helper.
pub(crate) fn colorcorrection_scroll_unit_delta(
    unit: gdk::ScrollUnit,
    delta_x: f64,
    delta_y: f64,
) -> Option<i32> {
    source_scroll_unit_delta(unit, delta_x, delta_y)
}

/// Ends a grid scroll sequence and clears the source-global remainder.
pub(crate) fn colorcorrection_scroll_end() {
    reset_source_scroll_units();
}

fn modifier_speed(modifiers: gdk::ModifierType) -> f64 {
    #[cfg(target_os = "macos")]
    let primary = gdk::ModifierType::META_MASK;
    #[cfg(not(target_os = "macos"))]
    let primary = gdk::ModifierType::CONTROL_MASK;
    let relevant = primary
        | gdk::ModifierType::SHIFT_MASK
        | gdk::ModifierType::CONTROL_MASK
        | gdk::ModifierType::ALT_MASK;
    let cleaned = modifiers & relevant;
    if cleaned == primary {
        0.1
    } else if cleaned == gdk::ModifierType::SHIFT_MASK
        || cleaned == primary | gdk::ModifierType::SHIFT_MASK
    {
        10.0
    } else {
        1.0
    }
}

fn draw_grid(
    cairo: &gtk4::cairo::Context,
    width: i32,
    height: i32,
    state: ColorCorrectionGridState,
    selected: Option<ColorCorrectionEndpoint>,
    saturation: f64,
) {
    let width = f64::from(width);
    let height = f64::from(height);
    cairo.set_source_rgb(0.2, 0.2, 0.2);
    let _ = cairo.paint();
    let inner_width = 2.0f64.mul_add(-COLORCORRECTION_GRID_INSET, width);
    let inner_height = 2.0f64.mul_add(-COLORCORRECTION_GRID_INSET, height);
    if inner_width <= 0.0 || inner_height <= 0.0 {
        return;
    }

    cairo.set_antialias(gtk4::cairo::Antialias::None);
    let cells = f64::from(u32::try_from(COLORCORRECTION_GRID_CELLS).expect("grid size is bounded"));
    for b_index in 0..COLORCORRECTION_GRID_CELLS {
        for a_index in 0..COLORCORRECTION_GRID_CELLS {
            let [red, green, blue] = grid_cell_srgb(saturation, a_index, b_index);
            cairo.set_source_rgb(red, green, blue);
            let x = COLORCORRECTION_GRID_INSET
                + inner_width * f64::from(u32::try_from(a_index).expect("grid index is bounded"))
                    / cells;
            let y = COLORCORRECTION_GRID_INSET + inner_height
                - inner_height
                    * f64::from(
                        u32::try_from(b_index + 1).expect("grid index increment is bounded"),
                    )
                    / cells;
            cairo.rectangle(x, y, inner_width / cells - 1.0, inner_height / cells - 1.0);
            let _ = cairo.fill();
        }
    }
    cairo.set_antialias(gtk4::cairo::Antialias::Default);

    let (shadow_x, shadow_y) = endpoint_screen_position(
        state,
        ColorCorrectionEndpoint::Shadows,
        inner_width,
        inner_height,
    );
    let (highlight_x, highlight_y) = endpoint_screen_position(
        state,
        ColorCorrectionEndpoint::Highlights,
        inner_width,
        inner_height,
    );
    cairo.set_line_width(2.0);
    cairo.set_source_rgb(0.6, 0.6, 0.6);
    cairo.move_to(shadow_x, shadow_y);
    cairo.line_to(highlight_x, highlight_y);
    let _ = cairo.stroke();

    draw_endpoint(
        cairo,
        shadow_x,
        shadow_y,
        selected == Some(ColorCorrectionEndpoint::Shadows),
        0.1,
    );
    draw_endpoint(
        cairo,
        highlight_x,
        highlight_y,
        selected == Some(ColorCorrectionEndpoint::Highlights),
        0.9,
    );
}

fn grid_cell_srgb(saturation: f64, a_index: usize, b_index: usize) -> [f64; 3] {
    let cells = f64::from(u32::try_from(COLORCORRECTION_GRID_CELLS).expect("grid size is bounded"));
    let a_position =
        f64::from(u32::try_from(a_index).expect("grid index is bounded")) / (cells - 1.0);
    let b_position =
        f64::from(u32::try_from(b_index).expect("grid index is bounded")) / (cells - 1.0);
    let chroma_scale = saturation * (53.390_011 * 0.05 * COLORCORRECTION_GRID_MAX);
    lab_d50_to_srgb(
        53.390_011,
        chroma_scale * (a_position - 0.5),
        chroma_scale * (b_position - 0.5),
    )
}

fn endpoint_screen_position(
    state: ColorCorrectionGridState,
    endpoint: ColorCorrectionEndpoint,
    width: f64,
    height: f64,
) -> (f64, f64) {
    let (a, b) = state.endpoint(endpoint);
    (
        (0.5 * width).mul_add(
            1.0 + a / COLORCORRECTION_GRID_MAX,
            COLORCORRECTION_GRID_INSET,
        ),
        (0.5 * height).mul_add(
            1.0 - b / COLORCORRECTION_GRID_MAX,
            COLORCORRECTION_GRID_INSET,
        ),
    )
}

fn draw_endpoint(cairo: &gtk4::cairo::Context, x: f64, y: f64, selected: bool, grey: f64) {
    cairo.set_source_rgb(grey, grey, grey);
    cairo.arc(
        x,
        y,
        if selected { 5.0 } else { 3.0 },
        0.0,
        std::f64::consts::TAU,
    );
    let _ = cairo.fill();
}

fn lab_d50_to_srgb(lightness: f64, opponent_a: f64, opponent_b: f64) -> [f64; 3] {
    let lab_y = (lightness + 16.0) / 116.0;
    let lab_x = lab_y + opponent_a / 500.0;
    let lab_z = lab_y - opponent_b / 200.0;
    let d50_x = 0.964_22 * lab_inverse(lab_x);
    let d50_y = lab_inverse(lab_y);
    let d50_z = 0.825_21 * lab_inverse(lab_z);
    let d65_x =
        0.955_576_6f64.mul_add(d50_x, (-0.023_039_3f64).mul_add(d50_y, 0.063_163_6 * d50_z));
    let d65_y =
        (-0.028_289_5f64).mul_add(d50_x, 1.009_941_6f64.mul_add(d50_y, 0.021_007_7 * d50_z));
    let d65_z = 0.012_298_2f64.mul_add(d50_x, (-0.020_483f64).mul_add(d50_y, 1.329_909_8 * d50_z));
    let linear_red = 3.240_454_2f64.mul_add(
        d65_x,
        (-1.537_138_5f64).mul_add(d65_y, -0.498_531_4 * d65_z),
    );
    let linear_green =
        (-0.969_266f64).mul_add(d65_x, 1.876_010_8f64.mul_add(d65_y, 0.041_556 * d65_z));
    let linear_blue =
        0.055_643_4f64.mul_add(d65_x, (-0.204_025_9f64).mul_add(d65_y, 1.057_225_2 * d65_z));
    [
        srgb_encode(linear_red),
        srgb_encode(linear_green),
        srgb_encode(linear_blue),
    ]
}

fn lab_inverse(value: f64) -> f64 {
    const DELTA: f64 = 6.0 / 29.0;
    if value > DELTA {
        value.powi(3)
    } else {
        3.0 * DELTA.powi(2) * (value - 4.0 / 29.0)
    }
}

fn srgb_encode(value: f64) -> f64 {
    let encoded = if value <= 0.003_130_8 {
        12.92 * value
    } else {
        1.055f64.mul_add(value.powf(1.0 / 2.4), -0.055)
    };
    encoded.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::{
        COLORCORRECTION_GRID_TOOLTIP, COLORCORRECTION_SATURATION, COLORCORRECTION_SOURCE_MAP,
        ColorCorrectionDoubleClick, ColorCorrectionEndpoint, ColorCorrectionGridModel,
        ColorCorrectionGridState, colorcorrection_scroll_end, colorcorrection_scroll_unit_delta,
        grid_cell_srgb, lab_d50_to_srgb, scrolled_saturation,
    };

    fn assert_exact_f64(actual: f64, expected: f64) {
        assert_eq!(actual.to_bits(), expected.to_bits());
    }

    fn assert_exact_f64_pair(actual: (f64, f64), expected: (f64, f64)) {
        assert_eq!(
            [actual.0.to_bits(), actual.1.to_bits()],
            [expected.0.to_bits(), expected.1.to_bits()]
        );
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() <= 1.0e-12,
            "expected {expected:?}, got {actual:?}"
        );
    }

    #[test]
    fn source_map_preserves_native_metadata_and_saturation_contract() {
        assert_eq!(COLORCORRECTION_SOURCE_MAP.title(), "color correction");
        assert_eq!(
            COLORCORRECTION_SOURCE_MAP.group_keys(),
            ["group.color", "group.grading"]
        );
        assert!(!COLORCORRECTION_SOURCE_MAP.default_enabled());
        assert!(!COLORCORRECTION_SOURCE_MAP.default_expanded());
        assert_eq!(COLORCORRECTION_SATURATION.parameter_id(), "saturation");
        assert_eq!(COLORCORRECTION_SATURATION.label(), "saturation");
        assert_exact_f64_pair(COLORCORRECTION_SATURATION.range(), (-3.0, 3.0));
        assert_exact_f64(COLORCORRECTION_SATURATION.default_value(), 1.0);
        assert_eq!(COLORCORRECTION_SATURATION.digits(), 2);
        assert_exact_f64(COLORCORRECTION_SATURATION.step(), 0.01);
        assert!(COLORCORRECTION_SATURATION.automatic_step());
        assert_eq!(
            COLORCORRECTION_GRID_TOOLTIP,
            "drag the line for split-toning. bright means highlights, dark means shadows. use mouse wheel to change saturation."
        );
    }

    #[test]
    fn pointer_hover_drag_and_ties_follow_native_endpoint_rules() {
        let mut model = ColorCorrectionGridModel::new(
            ColorCorrectionGridState::new(20.0, 20.0, -20.0, -20.0).expect("finite fixture"),
        );
        model.hover(30.0, 79.0, 110.0, 110.0);
        assert_eq!(model.selected(), Some(ColorCorrectionEndpoint::Shadows));
        let dragged = model
            .drag(5.0, 105.0, 110.0, 110.0)
            .expect("selected shadow endpoint");
        assert_exact_f64_pair((dragged.loa(), dragged.lob()), (-40.0, -40.0));
        assert_exact_f64_pair((dragged.hia(), dragged.hib()), (20.0, 20.0));

        let mut tied = ColorCorrectionGridModel::new(ColorCorrectionGridState::DEFAULT);
        tied.hover(55.0, 54.0, 110.0, 110.0);
        assert_eq!(
            tied.selected(),
            Some(ColorCorrectionEndpoint::Highlights),
            "equal distances select highlights because native uses <= for the bright endpoint"
        );
    }

    #[test]
    fn double_click_and_keys_reset_or_nudge_only_the_selected_endpoint() {
        let fixture = ColorCorrectionGridState::new(4.0, 5.0, -6.0, -7.0).expect("finite fixture");
        let mut model = ColorCorrectionGridModel::new(fixture);
        model.hover(50.0, 59.0, 110.0, 110.0);
        assert_eq!(model.selected(), Some(ColorCorrectionEndpoint::Shadows));
        let nudged = model.nudge(1.0, 0.0, 1.0).expect("selected endpoint");
        assert_exact_f64(nudged.loa(), -5.5);
        assert_exact_f64_pair((nudged.hia(), nudged.hib()), (4.0, 5.0));
        assert!(matches!(
            model.double_click(),
            ColorCorrectionDoubleClick::Endpoint(state)
                if [state.loa().to_bits(), state.lob().to_bits()]
                    == [0.0_f64.to_bits(), 0.0_f64.to_bits()]
                    && [state.hia().to_bits(), state.hib().to_bits()]
                        == [4.0_f64.to_bits(), 5.0_f64.to_bits()]
        ));

        let mut background = ColorCorrectionGridModel::new(fixture);
        assert_eq!(
            background.double_click(),
            ColorCorrectionDoubleClick::AllDefaults
        );
        let default_state = background.state();
        assert_exact_f64_pair((default_state.hia(), default_state.hib()), (0.0, 0.0));
        assert_exact_f64_pair((default_state.loa(), default_state.lob()), (0.0, 0.0));
    }

    #[test]
    fn wheel_changes_saturation_in_native_tenths_and_clamps() {
        assert_close(scrolled_saturation(1.0, 1), 0.9);
        assert_close(scrolled_saturation(1.0, -1), 1.1);
        assert_exact_f64(scrolled_saturation(2.95, -1), 3.0);
        assert_exact_f64(scrolled_saturation(-2.95, 1), -3.0);
        assert_exact_f64(scrolled_saturation(4.25, 1), 3.0);
    }

    #[test]
    fn native_scroll_sums_horizontal_and_vertical_units() {
        colorcorrection_scroll_end();
        assert_eq!(
            colorcorrection_scroll_unit_delta(gtk4::gdk::ScrollUnit::Wheel, 1.0, 0.0),
            Some(1),
            "horizontal wheel motion contributes one source unit"
        );
        assert_eq!(
            colorcorrection_scroll_unit_delta(gtk4::gdk::ScrollUnit::Wheel, 50.0, -1.0),
            Some(0),
            "opposing wheel axes emit a summed zero source unit"
        );
        #[cfg(target_os = "macos")]
        let surface_unit = 50.0;
        #[cfg(not(target_os = "macos"))]
        let surface_unit = 1.0;
        assert_eq!(
            colorcorrection_scroll_unit_delta(
                gtk4::gdk::ScrollUnit::Surface,
                surface_unit * 0.5,
                0.0,
            ),
            None
        );
        assert_eq!(
            colorcorrection_scroll_unit_delta(
                gtk4::gdk::ScrollUnit::Surface,
                surface_unit * 0.5,
                0.0,
            ),
            Some(1),
            "horizontal surface fractions accumulate into a source unit"
        );
        colorcorrection_scroll_end();
        assert_eq!(
            colorcorrection_scroll_unit_delta(
                gtk4::gdk::ScrollUnit::Surface,
                surface_unit,
                surface_unit,
            ),
            Some(2),
            "whole surface units from both axes are summed"
        );
        colorcorrection_scroll_end();
    }

    #[test]
    fn raw_imported_saturation_outlier_remains_visible_in_grid_colors() {
        let raw = grid_cell_srgb(4.25, 4, 4);
        let slider_clamped = grid_cell_srgb(3.0, 4, 4);
        assert!(
            raw.into_iter()
                .zip(slider_clamped)
                .any(|(raw, clamped)| (raw - clamped).abs() > 0.01),
            "native grid drawing must not silently substitute the slider's clamped value"
        );
    }

    #[test]
    fn source_neutral_lab_cell_converts_back_to_middle_grey() {
        let [red, green, blue] = lab_d50_to_srgb(53.390_011, 0.0, 0.0);
        for channel in [red, green, blue] {
            assert!((channel - 0.5).abs() < 0.002);
        }
    }
}
