//! Pure Color Zones graph preparation derived from `src/iop/colorzones.c`.
//!
//! This module emits presentation-independent paint data. Curve interpolation is
//! delegated to the processing plan, while source geometry, channel coordinates,
//! paint order, and display colors are prepared here for a later GTK adapter.

#![allow(
    clippy::cast_precision_loss,
    clippy::missing_errors_doc,
    reason = "bounded source grids and typed render-construction errors are explicit here"
)]

use std::fmt;

use rusttable_processing::{
    COLORZONES_CHANNELS, ColorZonesChannel, ColorZonesCompileError, ColorZonesConfig,
    ColorZonesParameterError, ColorZonesPlan,
};

use super::model::ColorZonesEditorState;

/// Native `DT_IOP_COLORZONES_RES` graph sample count.
pub const COLORZONES_GRAPH_RESOLUTION: usize = 256;
/// Native graph-height configuration minimum, in logical pixels.
pub const COLORZONES_GRAPH_HEIGHT_MIN: u16 = 100;
/// Native graph-height configuration maximum, in logical pixels.
pub const COLORZONES_GRAPH_HEIGHT_MAX: u16 = 300;
/// Native graph-height configuration default, in logical pixels.
pub const COLORZONES_GRAPH_HEIGHT_DEFAULT: u16 = 200;
/// Native logical inset around both graph drawing areas.
pub const COLORZONES_GRAPH_INSET: f32 = 5.0;
/// Native horizontal background-field resolution.
pub const COLORZONES_FIELD_COLUMNS: usize = 64;
/// Native vertical background-field resolution.
pub const COLORZONES_FIELD_ROWS: usize = 36;
/// Native background-saturation configuration minimum.
pub const COLORZONES_BACKGROUND_SATURATION_MIN: f32 = 0.1;
/// Native background-saturation configuration maximum.
pub const COLORZONES_BACKGROUND_SATURATION_MAX: f32 = 1.0;
/// Native background-saturation configuration default.
pub const COLORZONES_BACKGROUND_SATURATION_DEFAULT: f32 = 0.5;
/// Native curve stroke width in logical pixels.
pub const COLORZONES_CURVE_STROKE_WIDTH: f32 = 2.0;
/// Native inactive-curve opacity.
pub const COLORZONES_INACTIVE_CURVE_OPACITY: f32 = 0.3;
/// Native active-curve opacity.
pub const COLORZONES_ACTIVE_CURVE_OPACITY: f32 = 1.0;
/// Native knot and selection-ring stroke width in logical pixels.
pub const COLORZONES_RING_STROKE_WIDTH: f32 = 1.0;
/// Native ordinary knot-ring radius in logical pixels.
pub const COLORZONES_KNOT_RING_RADIUS: f32 = 3.0;
/// Native selected knot-ring radius in logical pixels.
pub const COLORZONES_SELECTED_RING_RADIUS: f32 = 4.0;
/// Native area-edit x-marker width in logical pixels.
pub const COLORZONES_AREA_MARKER_WIDTH: f32 = 7.0;

const LUT_LAST_INDEX: usize = 0xffff;
const GRAPH_LAST_INDEX: usize = COLORZONES_GRAPH_RESOLUTION - 1;
const GRAPH_LAST_INDEX_F32: f32 = 255.0;
const LUT_STRIDE: usize = LUT_LAST_INDEX / GRAPH_LAST_INDEX;
const CHROMA_NORMALIZATION: f32 = 128.0 * std::f32::consts::SQRT_2;

/// One of the eight source-named hue-band action landmarks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorZonesBandLandmark {
    pub name: &'static str,
    pub x: f32,
}

/// Native band names and their `element / 8` graph abscissae.
#[rustfmt::skip]
pub const COLORZONES_BAND_LANDMARKS: [ColorZonesBandLandmark; 8] = [
    ColorZonesBandLandmark { name: "red",     x: 0.0 },
    ColorZonesBandLandmark { name: "orange",  x: 0.125 },
    ColorZonesBandLandmark { name: "yellow",  x: 0.25 },
    ColorZonesBandLandmark { name: "green",   x: 0.375 },
    ColorZonesBandLandmark { name: "aqua",    x: 0.5 },
    ColorZonesBandLandmark { name: "blue",    x: 0.625 },
    ColorZonesBandLandmark { name: "purple",  x: 0.75 },
    ColorZonesBandLandmark { name: "magenta", x: 0.875 },
];

/// Checked source graph height in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorZonesGraphHeight(u16);

impl ColorZonesGraphHeight {
    #[must_use]
    pub const fn new(logical_pixels: u16) -> Option<Self> {
        if logical_pixels >= COLORZONES_GRAPH_HEIGHT_MIN
            && logical_pixels <= COLORZONES_GRAPH_HEIGHT_MAX
        {
            Some(Self(logical_pixels))
        } else {
            None
        }
    }

    /// Clamps an arbitrary value to the native logical-pixel bounds.
    #[must_use]
    pub fn clamped(logical_pixels: u16) -> Self {
        Self(logical_pixels.clamp(COLORZONES_GRAPH_HEIGHT_MIN, COLORZONES_GRAPH_HEIGHT_MAX))
    }

    #[must_use]
    pub const fn logical_pixels(self) -> u16 {
        self.0
    }
}

impl Default for ColorZonesGraphHeight {
    fn default() -> Self {
        Self(COLORZONES_GRAPH_HEIGHT_DEFAULT)
    }
}

/// Checked native background-saturation factor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorZonesBackgroundSaturation(f32);

impl ColorZonesBackgroundSaturation {
    #[must_use]
    pub fn new(value: f32) -> Option<Self> {
        (value.is_finite()
            && (COLORZONES_BACKGROUND_SATURATION_MIN..=COLORZONES_BACKGROUND_SATURATION_MAX)
                .contains(&value))
        .then_some(Self(value))
    }

    #[must_use]
    pub const fn value(self) -> f32 {
        self.0
    }
}

impl Default for ColorZonesBackgroundSaturation {
    fn default() -> Self {
        Self(COLORZONES_BACKGROUND_SATURATION_DEFAULT)
    }
}

/// Native zoom and pan mapping between curve and visible graph coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorZonesViewTransform {
    zoom_factor: f32,
    offset_x: f32,
    offset_y: f32,
}

impl ColorZonesViewTransform {
    /// Builds a source-compatible view, clamping finite offsets to the visible range.
    #[must_use]
    pub fn new(zoom_factor: f32, offset_x: f32, offset_y: f32) -> Option<Self> {
        if !zoom_factor.is_finite()
            || zoom_factor < 1.0
            || !offset_x.is_finite()
            || !offset_y.is_finite()
        {
            return None;
        }
        let maximum = (zoom_factor - 1.0) / zoom_factor;
        Some(Self {
            zoom_factor,
            offset_x: offset_x.clamp(0.0, maximum),
            offset_y: offset_y.clamp(0.0, maximum),
        })
    }

    #[must_use]
    pub const fn zoom_factor(self) -> f32 {
        self.zoom_factor
    }

    #[must_use]
    pub const fn offsets(self) -> (f32, f32) {
        (self.offset_x, self.offset_y)
    }

    #[must_use]
    pub fn curve_x_to_view(self, x: f32) -> f32 {
        (x - self.offset_x) * self.zoom_factor
    }

    #[must_use]
    pub fn curve_y_to_view(self, y: f32) -> f32 {
        (y - self.offset_y) * self.zoom_factor
    }

    #[must_use]
    pub fn view_x_to_curve(self, x: f32) -> f32 {
        x / self.zoom_factor + self.offset_x
    }

    #[must_use]
    pub fn view_y_to_curve(self, y: f32) -> f32 {
        y / self.zoom_factor + self.offset_y
    }
}

impl Default for ColorZonesViewTransform {
    fn default() -> Self {
        Self {
            zoom_factor: 1.0,
            offset_x: 0.0,
            offset_y: 0.0,
        }
    }
}

/// Source `LCh` coordinates (`L` and `C` in Lab units, hue in turns).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorZonesLch {
    pub lightness: f32,
    pub chroma: f32,
    pub hue: f32,
}

impl ColorZonesLch {
    /// Source fallback converted from the native sRGB `(0, 0.3, 0.7)` color.
    pub const SOURCE_FALLBACK: Self = Self {
        lightness: 33.911_79,
        chroma: 62.370_728,
        hue: 0.784_955_44,
    };
}

/// A normalized top-origin rectangle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorZonesRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// One source background-field cell, in row-major paint order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorZonesFieldCell {
    pub column: usize,
    pub row: usize,
    pub bounds: ColorZonesRect,
    pub lch: ColorZonesLch,
}

/// One source bottom-strip cell, in left-to-right paint order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorZonesBottomCell {
    pub column: usize,
    pub bounds: ColorZonesRect,
    pub lch: ColorZonesLch,
}

/// One graph-space point. Y remains bottom-origin like native curve coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorZonesGraphPoint {
    x: f32,
    y: f32,
}

impl ColorZonesGraphPoint {
    #[must_use]
    pub const fn x(self) -> f32 {
        self.x
    }

    #[must_use]
    pub const fn y(self) -> f32 {
        self.y
    }
}

/// One output curve sampled and styled for native paint order.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorZonesGraphCurve {
    channel: ColorZonesChannel,
    selected: bool,
    opacity: f32,
    points: [ColorZonesGraphPoint; COLORZONES_GRAPH_RESOLUTION],
}

impl ColorZonesGraphCurve {
    #[must_use]
    pub const fn channel(&self) -> ColorZonesChannel {
        self.channel
    }

    #[must_use]
    pub const fn selected(&self) -> bool {
        self.selected
    }

    #[must_use]
    pub const fn opacity(&self) -> f32 {
        self.opacity
    }

    #[must_use]
    pub const fn stroke_width(&self) -> f32 {
        COLORZONES_CURVE_STROKE_WIDTH
    }

    #[must_use]
    pub const fn points(&self) -> &[ColorZonesGraphPoint; COLORZONES_GRAPH_RESOLUTION] {
        &self.points
    }
}

/// One ordinary knot ring on the selected output curve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorZonesKnot {
    pub point: ColorZonesGraphPoint,
    pub radius: f32,
    pub stroke_width: f32,
}

/// Validated inputs that influence deterministic graph preparation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorZonesRenderOptions {
    pub graph_height: ColorZonesGraphHeight,
    pub background_saturation: ColorZonesBackgroundSaturation,
    pub view: ColorZonesViewTransform,
    pub base_color: ColorZonesLch,
}

impl Default for ColorZonesRenderOptions {
    fn default() -> Self {
        Self {
            graph_height: ColorZonesGraphHeight::default(),
            background_saturation: ColorZonesBackgroundSaturation::default(),
            view: ColorZonesViewTransform::default(),
            base_color: ColorZonesLch::SOURCE_FALLBACK,
        }
    }
}

/// Source-ordered graph and bottom-strip primitives.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorZonesRenderModel {
    plan: ColorZonesPlan,
    options: ColorZonesRenderOptions,
    field: Vec<ColorZonesFieldCell>,
    curves: [ColorZonesGraphCurve; COLORZONES_CHANNELS],
    knots: Vec<ColorZonesKnot>,
    bottom_strip: [ColorZonesBottomCell; COLORZONES_FIELD_COLUMNS],
}

impl ColorZonesRenderModel {
    /// Compiles graph primitives with native defaults.
    pub fn new(editor: &ColorZonesEditorState) -> Result<Self, ColorZonesRenderError> {
        Self::with_options(editor, ColorZonesRenderOptions::default())
    }

    /// Compiles graph primitives through the canonical processing spline plan.
    pub fn with_options(
        editor: &ColorZonesEditorState,
        options: ColorZonesRenderOptions,
    ) -> Result<Self, ColorZonesRenderError> {
        let config = ColorZonesConfig::try_from(editor.parameters())?;
        let plan = ColorZonesPlan::new(config)?;
        let selected = editor.output_channel();
        let curves = source_curve_order(selected).map(|channel| ColorZonesGraphCurve {
            channel,
            selected: channel == selected,
            opacity: if channel == selected {
                COLORZONES_ACTIVE_CURVE_OPACITY
            } else {
                COLORZONES_INACTIVE_CURVE_OPACITY
            },
            points: sample_channel(&plan, channel, options.view),
        });
        let knots = editor
            .active_nodes(selected)
            .iter()
            .map(|node| ColorZonesKnot {
                point: ColorZonesGraphPoint {
                    x: options.view.curve_x_to_view(node.x),
                    y: options.view.curve_y_to_view(node.y),
                },
                radius: COLORZONES_KNOT_RING_RADIUS,
                stroke_width: COLORZONES_RING_STROKE_WIDTH,
            })
            .collect();
        let field = build_field(editor.selection_channel(), selected, options);
        let bottom_strip = build_bottom_strip(editor.selection_channel(), options);
        Ok(Self {
            plan,
            options,
            field,
            curves,
            knots,
            bottom_strip,
        })
    }

    #[must_use]
    pub const fn options(&self) -> ColorZonesRenderOptions {
        self.options
    }

    #[must_use]
    pub fn field(&self) -> &[ColorZonesFieldCell] {
        &self.field
    }

    /// Curves in native paint order: two inactive curves, then the active curve.
    #[must_use]
    pub const fn curves(&self) -> &[ColorZonesGraphCurve; COLORZONES_CHANNELS] {
        &self.curves
    }

    #[must_use]
    pub fn knots(&self) -> &[ColorZonesKnot] {
        &self.knots
    }

    #[must_use]
    pub const fn bottom_strip(&self) -> &[ColorZonesBottomCell; COLORZONES_FIELD_COLUMNS] {
        &self.bottom_strip
    }

    /// Samples one curve in canonical curve coordinates, independent of the view.
    #[must_use]
    pub fn sample(&self, channel: ColorZonesChannel, x: f32) -> Option<f32> {
        x.is_finite()
            .then(|| self.plan.sample_curve(channel, x.clamp(0.0, 1.0)))
    }
}

/// Failure while preparing source-shaped graph data.
#[derive(Debug)]
pub enum ColorZonesRenderError {
    Parameters(ColorZonesParameterError),
    Compile(ColorZonesCompileError),
}

impl fmt::Display for ColorZonesRenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parameters(error) => {
                write!(formatter, "invalid Color Zones graph parameters: {error}")
            }
            Self::Compile(error) => {
                write!(formatter, "failed to compile Color Zones graph: {error}")
            }
        }
    }
}

impl std::error::Error for ColorZonesRenderError {}

impl From<ColorZonesParameterError> for ColorZonesRenderError {
    fn from(error: ColorZonesParameterError) -> Self {
        Self::Parameters(error)
    }
}

impl From<ColorZonesCompileError> for ColorZonesRenderError {
    fn from(error: ColorZonesCompileError) -> Self {
        Self::Compile(error)
    }
}

fn source_curve_order(selected: ColorZonesChannel) -> [ColorZonesChannel; COLORZONES_CHANNELS] {
    let channels = [
        ColorZonesChannel::Lightness,
        ColorZonesChannel::Chroma,
        ColorZonesChannel::Hue,
    ];
    let index = selected.index();
    [
        channels[(index + 1) % 3],
        channels[(index + 2) % 3],
        channels[index],
    ]
}

fn sample_channel(
    plan: &ColorZonesPlan,
    channel: ColorZonesChannel,
    view: ColorZonesViewTransform,
) -> [ColorZonesGraphPoint; COLORZONES_GRAPH_RESOLUTION] {
    let lut = plan.lut(channel);
    std::array::from_fn(|index| {
        let numerator = u16::try_from(index).expect("Color Zones graph index fits u16");
        let curve_x = f32::from(numerator) / GRAPH_LAST_INDEX_F32;
        ColorZonesGraphPoint {
            // Native starts each path at the left edge before transforming k=1..255.
            x: if index == 0 {
                0.0
            } else {
                view.curve_x_to_view(curve_x)
            },
            y: view.curve_y_to_view(lut[index * LUT_STRIDE]),
        }
    })
}

fn build_field(
    selection: ColorZonesChannel,
    output: ColorZonesChannel,
    options: ColorZonesRenderOptions,
) -> Vec<ColorZonesFieldCell> {
    let mut cells = Vec::with_capacity(COLORZONES_FIELD_COLUMNS * COLORZONES_FIELD_ROWS);
    for row in 0..COLORZONES_FIELD_ROWS {
        for column in 0..COLORZONES_FIELD_COLUMNS {
            let lch = field_lch(selection, output, options, column, row);
            cells.push(ColorZonesFieldCell {
                column,
                row,
                bounds: ColorZonesRect {
                    x: column as f32 / COLORZONES_FIELD_COLUMNS as f32,
                    y: row as f32 / COLORZONES_FIELD_ROWS as f32,
                    width: 1.0 / COLORZONES_FIELD_COLUMNS as f32,
                    height: 1.0 / COLORZONES_FIELD_ROWS as f32,
                },
                lch,
            });
        }
    }
    cells
}

fn field_lch(
    selection: ColorZonesChannel,
    output: ColorZonesChannel,
    options: ColorZonesRenderOptions,
    column: usize,
    row: usize,
) -> ColorZonesLch {
    let i = column as f32;
    let j = row as f32;
    let view = options.view;
    let ii = view.view_x_to_curve((i + 0.5) / (COLORZONES_FIELD_COLUMNS - 1) as f32);
    let iih = view.view_x_to_curve(i / (COLORZONES_FIELD_COLUMNS - 1) as f32);
    let jj = view.view_y_to_curve(1.0 - (j - 0.5) / (COLORZONES_FIELD_ROWS - 1) as f32);
    let jjh = view.view_y_to_curve(1.0 - j / (COLORZONES_FIELD_ROWS - 1) as f32) + 0.5;
    let base = options.base_color;
    let saturation = options.background_saturation.value();
    let mut lch = match selection {
        ColorZonesChannel::Lightness => ColorZonesLch {
            lightness: 100.0 * ii,
            chroma: CHROMA_NORMALIZATION * saturation * 0.5,
            hue: base.hue,
        },
        ColorZonesChannel::Chroma => ColorZonesLch {
            lightness: 50.0,
            chroma: base.chroma * 2.0 * saturation * ii,
            hue: base.hue,
        },
        ColorZonesChannel::Hue => ColorZonesLch {
            lightness: 50.0,
            chroma: CHROMA_NORMALIZATION * saturation * 0.5,
            hue: iih,
        },
    };
    match output {
        ColorZonesChannel::Lightness if selection == ColorZonesChannel::Lightness => {
            lch.lightness *= jj;
        }
        ColorZonesChannel::Lightness => lch.lightness += -50.0 + 100.0 * jj,
        ColorZonesChannel::Chroma => lch.chroma *= 2.0 * jj,
        ColorZonesChannel::Hue => lch.hue += jjh,
    }
    lch
}

fn build_bottom_strip(
    selection: ColorZonesChannel,
    options: ColorZonesRenderOptions,
) -> [ColorZonesBottomCell; COLORZONES_FIELD_COLUMNS] {
    std::array::from_fn(|column| {
        let i = column as f32;
        let ii = options
            .view
            .view_x_to_curve((i + 0.5) / (COLORZONES_FIELD_COLUMNS - 1) as f32);
        let iih = options
            .view
            .view_x_to_curve(i / (COLORZONES_FIELD_COLUMNS - 1) as f32);
        let base = options.base_color;
        let lch = match selection {
            ColorZonesChannel::Lightness => ColorZonesLch {
                lightness: 100.0 * ii,
                chroma: CHROMA_NORMALIZATION * 0.5,
                hue: base.hue,
            },
            ColorZonesChannel::Chroma => ColorZonesLch {
                lightness: 50.0,
                chroma: base.chroma * 2.0 * ii,
                hue: base.hue,
            },
            ColorZonesChannel::Hue => ColorZonesLch {
                lightness: 50.0,
                chroma: CHROMA_NORMALIZATION * 0.5,
                hue: iih,
            },
        };
        ColorZonesBottomCell {
            column,
            bounds: ColorZonesRect {
                x: i / COLORZONES_FIELD_COLUMNS as f32,
                y: 0.0,
                width: 1.0 / COLORZONES_FIELD_COLUMNS as f32,
                height: 1.0,
            },
            lch,
        }
    })
}

#[cfg(test)]
#[rustfmt::skip]
#[allow(clippy::float_cmp, reason = "source constants and exact affine transforms are compared directly")]
mod tests {
    use rusttable_processing::{ColorZonesChannel, ColorZonesNode, ColorZonesSplinesVersion};
    use super::*;

    fn options(view: ColorZonesViewTransform, base_color: ColorZonesLch) -> ColorZonesRenderOptions {
        ColorZonesRenderOptions {
            graph_height: ColorZonesGraphHeight::default(),
            background_saturation: ColorZonesBackgroundSaturation::default(),
            view,
            base_color,
        }
    }

    #[test]
    fn source_dimensions_ranges_and_landmarks_are_exact() {
        assert_eq!((COLORZONES_GRAPH_HEIGHT_MIN, COLORZONES_GRAPH_HEIGHT_MAX, COLORZONES_GRAPH_HEIGHT_DEFAULT), (100, 300, 200));
        assert_eq!((COLORZONES_GRAPH_INSET, COLORZONES_AREA_MARKER_WIDTH), (5.0, 7.0));
        assert_eq!((COLORZONES_FIELD_COLUMNS, COLORZONES_FIELD_ROWS, COLORZONES_GRAPH_RESOLUTION), (64, 36, 256));
        assert!(ColorZonesGraphHeight::new(99).is_none());
        assert_eq!(ColorZonesGraphHeight::default().logical_pixels(), 200);
        assert_eq!(ColorZonesGraphHeight::clamped(1).logical_pixels(), 100);
        assert_eq!(ColorZonesGraphHeight::clamped(999).logical_pixels(), 300);
        assert!(ColorZonesBackgroundSaturation::new(0.099).is_none());
        assert_eq!(ColorZonesBackgroundSaturation::default().value().to_bits(), 0.5_f32.to_bits());
        assert_eq!(COLORZONES_BAND_LANDMARKS[0], ColorZonesBandLandmark { name: "red", x: 0.0 });
        assert_eq!(COLORZONES_BAND_LANDMARKS[7], ColorZonesBandLandmark { name: "magenta", x: 0.875 });
    }

    #[test]
    fn curves_use_source_order_opacity_resolution_and_strokes() {
        let editor = ColorZonesEditorState::with_output_channel(ColorZonesChannel::Chroma);
        let render = ColorZonesRenderModel::new(&editor).unwrap();
        let summary = std::array::from_fn(|index| {
            let curve = &render.curves()[index];
            (curve.channel(), curve.selected(), curve.opacity())
        });
        assert_eq!(summary, [(ColorZonesChannel::Hue, false, 0.3), (ColorZonesChannel::Lightness, false, 0.3), (ColorZonesChannel::Chroma, true, 1.0)]);
        assert!(render.curves().iter().all(|curve| curve.stroke_width() == 2.0 && curve.points().len() == 256));
        assert_eq!(render.curves()[2].points()[255].x().to_bits(), 1.0_f32.to_bits());
        assert_eq!((COLORZONES_RING_STROKE_WIDTH, COLORZONES_KNOT_RING_RADIUS, COLORZONES_SELECTED_RING_RADIUS), (1.0, 3.0, 4.0));
    }

    #[test]
    fn view_transform_clamps_offsets_and_drives_curves_knots_and_cells() {
        let view = ColorZonesViewTransform::new(2.0, 9.0, 0.25).unwrap();
        assert_eq!(view.offsets(), (0.5, 0.25));
        assert_eq!(view.curve_x_to_view(0.75), 0.5);
        assert_eq!(view.view_y_to_curve(0.5), 0.5);
        let base = ColorZonesLch { lightness: 40.0, chroma: 30.0, hue: 0.25 };
        let render = ColorZonesRenderModel::with_options(&ColorZonesEditorState::default(), options(view, base)).unwrap();
        assert_eq!(render.curves()[0].points()[0].x().to_bits(), 0);
        assert_eq!(render.knots()[0].point.x().to_bits(), (-0.5_f32).to_bits());
        assert_eq!(render.knots()[0].radius, 3.0);
        assert_eq!(render.field().len(), 64 * 36);
    }

    #[test]
    fn field_channel_coordinates_follow_native_half_cell_equations() {
        let base = ColorZonesLch { lightness: 40.0, chroma: 30.0, hue: 0.25 };
        let options = options(ColorZonesViewTransform::default(), base);
        let lch = field_lch(ColorZonesChannel::Lightness, ColorZonesChannel::Lightness, options, 0, 0);
        let (ii, jj) = (0.5 / 63.0, 1.0 - (-0.5 / 35.0));
        assert!((lch.lightness - 100.0 * ii * jj).abs() < 1.0e-6);
        assert_eq!(lch.chroma.to_bits(), (CHROMA_NORMALIZATION * 0.25).to_bits());
        assert_eq!(lch.hue.to_bits(), base.hue.to_bits());
        let hue = field_lch(ColorZonesChannel::Hue, ColorZonesChannel::Hue, options, 0, 0);
        assert_eq!(hue.hue.to_bits(), 1.5_f32.to_bits());
    }

    #[test]
    fn bottom_strip_is_64_cells_and_ignores_background_saturation() {
        let editor = ColorZonesEditorState::default();
        let low = ColorZonesRenderOptions {
            background_saturation: ColorZonesBackgroundSaturation::new(0.1).unwrap(),
            ..ColorZonesRenderOptions::default()
        };
        let high = ColorZonesRenderOptions {
            background_saturation: ColorZonesBackgroundSaturation::new(1.0).unwrap(),
            ..low
        };
        let low = ColorZonesRenderModel::with_options(&editor, low).unwrap();
        let high = ColorZonesRenderModel::with_options(&editor, high).unwrap();
        assert_eq!(low.bottom_strip(), high.bottom_strip());
        assert_eq!(low.bottom_strip()[0].bounds, ColorZonesRect { x: 0.0, y: 0.0, width: 1.0 / 64.0, height: 1.0 });
        assert_ne!(low.field()[0].lch.chroma.to_bits(), high.field()[0].lch.chroma.to_bits());
    }

    #[test]
    fn rendering_uses_processing_splines_for_both_versions() {
        let mut editor = ColorZonesEditorState::default();
        editor.set_output_channel(ColorZonesChannel::Hue);
        editor.insert_node(0.5, 0.8).unwrap();
        assert!(ColorZonesRenderModel::new(&editor).unwrap().sample(ColorZonesChannel::Hue, 0.5).unwrap() > 0.79);
        editor.set_splines_version(ColorZonesSplinesVersion::V1);
        editor.insert_node_on(ColorZonesChannel::Hue, 0.5, 0.8).unwrap();
        let render = ColorZonesRenderModel::new(&editor).unwrap();
        assert!(render.sample(ColorZonesChannel::Hue, 0.5).unwrap() > 0.79);
        assert_eq!(editor.active_nodes(ColorZonesChannel::Hue), [ColorZonesNode::new(0.0, 0.5), ColorZonesNode::new(0.5, 0.8), ColorZonesNode::new(1.0, 0.5)]);
    }
}
