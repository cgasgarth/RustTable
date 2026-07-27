//! Cairo paint adapter for Color Zones primitives derived from
//! Darktable `src/iop/colorzones.c` lines 1033-1677.

#![allow(
    clippy::cast_precision_loss,
    clippy::many_single_char_names,
    reason = "paint coordinates directly mirror the bounded native grids and Lab notation"
)]

use gtk4::{
    cairo::{Antialias, Context},
    gdk::RGBA,
};

use super::{
    COLORZONES_AREA_MARKER_WIDTH, COLORZONES_GRAPH_INSET, COLORZONES_SELECTED_RING_RADIUS,
    ColorZonesGraphCurve, ColorZonesInteraction, ColorZonesLch, ColorZonesRect,
    ColorZonesRenderModel, ColorZonesRenderOptions, ColorZonesSelection, ColorZonesViewTransform,
};

const GRAPH_OVERLAY_FALLBACK: [f64; 4] = [1.0, 0.0, 0.0, 1.0];
const GRAPH_BORDER: f64 = 0.1;
const GRAPH_INTERIOR: f64 = 0.3;
const GRAPH_KNOT: f64 = 0.6;
const GRAPH_SELECTED: f64 = 0.9;

pub(super) fn paint_graph(
    cairo: &Context,
    width: i32,
    height: i32,
    interaction: &ColorZonesInteraction,
    graph_overlay: Option<&RGBA>,
) {
    clear(cairo, graph_overlay);
    let Some(frame) = PaintFrame::new(width, height) else {
        return;
    };
    paint_frame_background(cairo, frame);

    let options = render_options(interaction);
    let Ok(model) = ColorZonesRenderModel::with_options(interaction.editor(), options) else {
        return;
    };

    cairo.set_antialias(Antialias::None);
    for cell in model.field() {
        set_lch_source(cairo, cell.lch);
        frame.rectangle(cairo, cell.bounds);
        let _ = cairo.fill();
    }
    cairo.set_antialias(Antialias::Default);

    if interaction.edit_by_area() {
        paint_area_markers(cairo, frame, interaction);
    }
    for curve in model.curves() {
        paint_curve(cairo, frame, curve);
    }
    paint_knots(cairo, frame, &model);
    if interaction.area_feedback_visible() {
        paint_area_feedback(cairo, frame, interaction, options);
    } else if let ColorZonesSelection::Node(index) = interaction.selection() {
        paint_selected_knot(cairo, frame, &model, index);
    }
}

pub(super) fn paint_bottom_strip(
    cairo: &Context,
    width: i32,
    height: i32,
    interaction: &ColorZonesInteraction,
    graph_overlay: Option<&RGBA>,
) {
    clear(cairo, graph_overlay);
    let Some(frame) = PaintFrame::new(width, height) else {
        return;
    };
    paint_frame_background(cairo, frame);
    let Ok(model) =
        ColorZonesRenderModel::with_options(interaction.editor(), render_options(interaction))
    else {
        return;
    };
    cairo.set_antialias(Antialias::None);
    for cell in model.bottom_strip() {
        set_lch_source(cairo, cell.lch);
        frame.rectangle(cairo, cell.bounds);
        let _ = cairo.fill();
    }
    cairo.set_antialias(Antialias::Default);
}

fn paint_frame_background(cairo: &Context, frame: PaintFrame) {
    cairo.set_line_width(1.0);
    cairo.set_source_rgb(GRAPH_BORDER, GRAPH_BORDER, GRAPH_BORDER);
    cairo.rectangle(frame.x, frame.y, frame.width, frame.height);
    let _ = cairo.stroke();

    cairo.set_source_rgb(GRAPH_INTERIOR, GRAPH_INTERIOR, GRAPH_INTERIOR);
    cairo.rectangle(frame.x, frame.y, frame.width, frame.height);
    let _ = cairo.fill();
}

fn render_options(interaction: &ColorZonesInteraction) -> ColorZonesRenderOptions {
    let (offset_x, offset_y) = interaction.offsets();
    let view = ColorZonesViewTransform::new(interaction.zoom_factor(), offset_x, offset_y)
        .expect("validated interaction view");
    ColorZonesRenderOptions {
        view,
        ..ColorZonesRenderOptions::default()
    }
}

fn paint_curve(cairo: &Context, frame: PaintFrame, curve: &ColorZonesGraphCurve) {
    let mut points = curve.points().iter();
    let Some(first) = points.next() else {
        return;
    };
    cairo.move_to(frame.screen_x(first.x()), frame.screen_y(first.y()));
    for point in points {
        cairo.line_to(frame.screen_x(point.x()), frame.screen_y(point.y()));
    }
    cairo.set_line_width(f64::from(curve.stroke_width()));
    cairo.set_source_rgba(0.7, 0.7, 0.7, f64::from(curve.opacity()));
    let _ = cairo.stroke();
}

fn paint_knots(cairo: &Context, frame: PaintFrame, model: &ColorZonesRenderModel) {
    for knot in model.knots() {
        cairo.set_line_width(f64::from(knot.stroke_width));
        cairo.set_source_rgb(GRAPH_KNOT, GRAPH_KNOT, GRAPH_KNOT);
        cairo.arc(
            frame.screen_x(knot.point.x()),
            frame.screen_y(knot.point.y()),
            f64::from(knot.radius),
            0.0,
            std::f64::consts::TAU,
        );
        let _ = cairo.stroke();
    }
}

fn paint_selected_knot(
    cairo: &Context,
    frame: PaintFrame,
    model: &ColorZonesRenderModel,
    index: usize,
) {
    let Some(knot) = model.knots().get(index) else {
        return;
    };
    cairo.set_source_rgb(GRAPH_SELECTED, GRAPH_SELECTED, GRAPH_SELECTED);
    cairo.set_line_width(f64::from(knot.stroke_width));
    cairo.arc(
        frame.screen_x(knot.point.x()),
        frame.screen_y(knot.point.y()),
        f64::from(COLORZONES_SELECTED_RING_RADIUS),
        0.0,
        std::f64::consts::TAU,
    );
    let _ = cairo.stroke();
}

fn paint_area_markers(cairo: &Context, frame: PaintFrame, interaction: &ColorZonesInteraction) {
    let selected = interaction.area_marker();
    let channel = interaction.editor().output_channel();
    for (index, node) in interaction
        .editor()
        .active_nodes(channel)
        .iter()
        .enumerate()
    {
        let x = frame.screen_x(interaction.curve_to_view(node.x, interaction.offsets().0));
        let y = frame.y + frame.height + f64::from(COLORZONES_GRAPH_INSET) - 1.0;
        let half = f64::from(COLORZONES_AREA_MARKER_WIDTH) * 0.5;
        cairo.move_to(x, y);
        cairo.line_to(x - half, y);
        cairo.line_to(x, y - f64::from(COLORZONES_AREA_MARKER_WIDTH));
        cairo.line_to(x + half, y);
        cairo.close_path();
        cairo.set_source_rgb(GRAPH_KNOT, GRAPH_KNOT, GRAPH_KNOT);
        cairo.set_line_width(1.0);
        if selected == Some(index) {
            let _ = cairo.fill();
        } else {
            let _ = cairo.stroke();
        }
    }
}

fn paint_area_feedback(
    cairo: &Context,
    frame: PaintFrame,
    interaction: &ColorZonesInteraction,
    options: ColorZonesRenderOptions,
) {
    let Some((pointer_x, _)) = interaction.pointer() else {
        return;
    };
    let Some((upper, lower)) = area_limit_curves(interaction, pointer_x, options) else {
        return;
    };
    let mut upper_points = upper.points().iter();
    let Some(first) = upper_points.next() else {
        return;
    };
    cairo.move_to(frame.screen_x(first.x()), frame.screen_y(first.y()));
    for point in upper_points {
        cairo.line_to(frame.screen_x(point.x()), frame.screen_y(point.y()));
    }
    for point in lower.points().iter().rev() {
        cairo.line_to(frame.screen_x(point.x()), frame.screen_y(point.y()));
    }
    cairo.close_path();
    cairo.set_source_rgba(0.7, 0.7, 0.7, 0.6);
    let _ = cairo.fill();

    let channel = interaction.editor().output_channel();
    let base = ColorZonesRenderModel::with_options(interaction.editor(), options).ok();
    let curve_x = interaction.view_to_curve(pointer_x, interaction.offsets().0);
    let curve_y = base
        .as_ref()
        .and_then(|model| model.sample(channel, curve_x))
        .map_or(0.5, |value| {
            interaction.curve_to_view(value, interaction.offsets().1)
        });
    cairo.set_source_rgba(0.9, 0.9, 0.9, 0.5);
    cairo.set_line_width(1.0);
    cairo.arc(
        frame.screen_x(pointer_x),
        frame.screen_y(curve_y),
        f64::from(interaction.area_radius()) * frame.width,
        0.0,
        std::f64::consts::TAU,
    );
    let _ = cairo.stroke();
}

fn area_limit_curves(
    interaction: &ColorZonesInteraction,
    pointer_x: f32,
    options: ColorZonesRenderOptions,
) -> Option<(ColorZonesGraphCurve, ColorZonesGraphCurve)> {
    let channel = interaction.editor().output_channel();
    let mut upper = interaction.clone();
    upper.set_pointer(pointer_x, 1.0).ok()?;
    upper.apply_gaussian_area_edit().ok()?;
    let upper = ColorZonesRenderModel::with_options(upper.editor(), options)
        .ok()?
        .curves()
        .iter()
        .find(|curve| curve.channel() == channel)?
        .clone();

    let mut lower = interaction.clone();
    lower.set_pointer(pointer_x, 0.0).ok()?;
    lower.apply_gaussian_area_edit().ok()?;
    let lower = ColorZonesRenderModel::with_options(lower.editor(), options)
        .ok()?
        .curves()
        .iter()
        .find(|curve| curve.channel() == channel)?
        .clone();
    Some((upper, lower))
}

fn clear(cairo: &Context, graph_overlay: Option<&RGBA>) {
    let [red, green, blue, alpha] = graph_overlay_channels(graph_overlay);
    cairo.set_source_rgba(red, green, blue, alpha);
    let _ = cairo.paint();
}

fn graph_overlay_channels(graph_overlay: Option<&RGBA>) -> [f64; 4] {
    graph_overlay.map_or(GRAPH_OVERLAY_FALLBACK, |color| {
        [
            f64::from(color.red()),
            f64::from(color.green()),
            f64::from(color.blue()),
            f64::from(color.alpha()),
        ]
    })
}

#[derive(Debug, Clone, Copy)]
struct PaintFrame {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl PaintFrame {
    fn new(width: i32, height: i32) -> Option<Self> {
        let inset = f64::from(COLORZONES_GRAPH_INSET);
        let width = f64::from(width) - 2.0 * inset;
        let height = f64::from(height) - 2.0 * inset;
        (width > 0.0 && height > 0.0).then_some(Self {
            x: inset,
            y: inset,
            width,
            height,
        })
    }

    fn rectangle(self, cairo: &Context, bounds: ColorZonesRect) {
        cairo.rectangle(
            self.x + f64::from(bounds.x) * self.width,
            self.y + f64::from(bounds.y) * self.height,
            f64::from(bounds.width) * self.width,
            f64::from(bounds.height) * self.height,
        );
    }

    fn screen_x(self, x: f32) -> f64 {
        self.x + f64::from(x) * self.width
    }

    fn screen_y(self, y: f32) -> f64 {
        self.y + (1.0 - f64::from(y)) * self.height
    }
}

fn set_lch_source(cairo: &Context, color: ColorZonesLch) {
    let [red, green, blue] = lch_d50_to_srgb(color);
    cairo.set_source_rgb(red, green, blue);
}

fn lch_d50_to_srgb(color: ColorZonesLch) -> [f64; 3] {
    let hue = f64::from(color.hue) * std::f64::consts::TAU;
    let mut lightness = f64::from(color.lightness);
    let mut opponent_a = f64::from(color.chroma) * hue.cos();
    let mut opponent_b = f64::from(color.chroma) * hue.sin();

    let original_lightness = lightness;
    let capped = lightness.min(100.0);
    let clip =
        1.0 - (capped - original_lightness) * 0.01 * (lightness - 20.0).clamp(0.0, 80.0) / 80.0;
    lightness = capped;
    if original_lightness.abs() > f64::EPSILON {
        let chroma_scale = lightness / original_lightness * clip.powi(3);
        opponent_a *= chroma_scale;
        opponent_b *= chroma_scale;
    }

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
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    encoded.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use gtk4::cairo::{Format, ImageSurface};

    use super::*;

    type Painter = fn(&Context, i32, i32, &ColorZonesInteraction, Option<&RGBA>);

    fn exterior_pixel(painter: Painter, graph_overlay: Option<&RGBA>) -> u32 {
        let surface = ImageSurface::create(Format::ARgb32, 32, 24).expect("ARGB32 paint surface");
        let cairo = Context::new(&surface).expect("Cairo paint context");
        painter(
            &cairo,
            surface.width(),
            surface.height(),
            &ColorZonesInteraction::default(),
            graph_overlay,
        );
        drop(cairo);

        let mut bytes = [0; 4];
        surface
            .with_data(|data| bytes.copy_from_slice(&data[..4]))
            .expect("painted surface data");
        u32::from_ne_bytes(bytes)
    }

    #[test]
    fn graph_overlay_preserves_the_widget_resolved_rgba() {
        let resolved = RGBA::new(0.2, 0.4, 0.6, 0.8);

        assert_eq!(
            graph_overlay_channels(Some(&resolved)).map(f64::to_bits),
            [
                f64::from(resolved.red()),
                f64::from(resolved.green()),
                f64::from(resolved.blue()),
                f64::from(resolved.alpha()),
            ]
            .map(f64::to_bits)
        );
    }

    #[test]
    fn both_source_callbacks_paint_the_resolved_grey_50_exterior() {
        let resolved = RGBA::new(119.0 / 255.0, 119.0 / 255.0, 119.0 / 255.0, 1.0);

        for painter in [paint_graph as Painter, paint_bottom_strip as Painter] {
            assert_eq!(exterior_pixel(painter, Some(&resolved)), 0xff77_7777);
        }
    }

    #[test]
    fn missing_graph_overlay_uses_the_source_opaque_red_fallback() {
        assert_eq!(
            graph_overlay_channels(None).map(f64::to_bits),
            [1.0_f64, 0.0, 0.0, 1.0].map(f64::to_bits)
        );
        for painter in [paint_graph as Painter, paint_bottom_strip as Painter] {
            assert_eq!(exterior_pixel(painter, None), 0xffff_0000);
        }
    }

    #[test]
    fn paint_frame_keeps_the_source_five_pixel_inset_at_any_strip_height() {
        let graph = PaintFrame::new(100, 200).expect("source-sized graph frame");
        assert_eq!(
            (graph.x, graph.y, graph.width, graph.height),
            (5.0, 5.0, 90.0, 190.0)
        );

        for (allocation_height, interior_height) in [(12, 2.0), (18, 8.0), (24, 14.0)] {
            let strip = PaintFrame::new(100, allocation_height)
                .expect("expanding font-sized bottom strip frame");
            assert_eq!(
                (strip.x, strip.y, strip.width, strip.height),
                (5.0, 5.0, 90.0, interior_height)
            );
        }
        assert!(PaintFrame::new(10, 24).is_none());
        assert!(PaintFrame::new(100, 10).is_none());
    }
}
