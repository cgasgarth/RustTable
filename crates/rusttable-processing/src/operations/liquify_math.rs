use super::{
    INVERSE_ITERATIONS, INVERSE_TOLERANCE, LIQUIFY_INTERPOLATION_POINTS, LIQUIFY_STAMP_RELOCATION,
    LiquifyConfig, LiquifyExecutionError, LiquifyInterpolation, LiquifyNode, LiquifyPathKind,
    LiquifyPoint, LiquifyValidationError, LiquifyWarpType, RasterDimensions, Stamp,
};
use crate::{FiniteF32, LinearRgb};
use rusttable_image::Roi;
use sha2::{Digest, Sha256};

pub(super) fn ensure_roi(
    roi: Roi,
    dimensions: RasterDimensions,
) -> Result<(), LiquifyExecutionError> {
    if roi.right() > dimensions.width() || roi.bottom() > dimensions.height() {
        return Err(LiquifyExecutionError::ArithmeticOverflow);
    }
    Ok(())
}

pub(super) fn transform_points<
    F: Fn(LiquifyPoint) -> Result<LiquifyPoint, LiquifyExecutionError>,
>(
    points: &mut [f32],
    transform: F,
) -> Result<(), LiquifyExecutionError> {
    if !points.len().is_multiple_of(2) {
        return Err(LiquifyExecutionError::ArithmeticOverflow);
    }
    for pair in points.as_chunks_mut::<2>().0 {
        let point =
            LiquifyPoint::new(pair[0], pair[1]).map_err(LiquifyExecutionError::Validation)?;
        let transformed = transform(point)?;
        pair[0] = transformed.x();
        pair[1] = transformed.y();
    }
    Ok(())
}

pub(super) fn expand_stamps(nodes: &[LiquifyNode]) -> Result<Vec<Stamp>, LiquifyExecutionError> {
    let mut stamps = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
        if node.path == LiquifyPathKind::Invalidated {
            break;
        }
        if node.path == LiquifyPathKind::MoveToV1 {
            if node.next < 0 {
                stamps.push(stamp(node, false));
            }
            continue;
        }
        let Some(previous) = index
            .checked_sub(1)
            .and_then(|position| nodes.get(position))
        else {
            return Err(LiquifyExecutionError::Validation(
                LiquifyValidationError::InvalidBezierTopology,
            ));
        };
        let length = match node.path {
            LiquifyPathKind::LineToV1 => previous.point.distance(node.point),
            LiquifyPathKind::CurveToV1 => {
                let samples = cubic_samples(previous.point, previous.ctrl2, node.ctrl1, node.point);
                samples
                    .windows(2)
                    .map(|pair| pair[0].distance(pair[1]))
                    .sum()
            }
            _ => 0.0,
        };
        if !length.is_finite() || length <= f32::EPSILON {
            return Err(LiquifyExecutionError::Validation(
                LiquifyValidationError::InvalidBezierTopology,
            ));
        }
        let mut distance = 0.0;
        while distance < length {
            let t = distance / length;
            let center = if node.path == LiquifyPathKind::LineToV1 {
                LiquifyPoint::new(
                    lerp(previous.point.x(), node.point.x(), t),
                    lerp(previous.point.y(), node.point.y(), t),
                )
                .expect("finite line sample")
            } else {
                point_at_length(
                    &cubic_samples(previous.point, previous.ctrl2, node.ctrl1, node.point),
                    distance,
                )
            };
            stamps.push(interpolated_stamp(previous, node, center, t)?);
            let radius = lerp(
                previous.radius.distance(previous.point),
                node.radius.distance(node.point),
                t,
            );
            distance += radius * LIQUIFY_STAMP_RELOCATION;
        }
    }
    Ok(stamps)
}

fn stamp(node: &LiquifyNode, relocated: bool) -> Stamp {
    Stamp {
        center: node.point,
        strength: node.strength,
        radius: node.radius.distance(node.point),
        control1: node.control1(),
        control2: node.control2(),
        warp_type: node.warp_type,
        relocated,
    }
}

fn interpolated_stamp(
    first: &LiquifyNode,
    second: &LiquifyNode,
    center: LiquifyPoint,
    t: f32,
) -> Result<Stamp, LiquifyExecutionError> {
    let first_vector = first
        .strength
        .sub(first.point)
        .map_err(LiquifyExecutionError::Validation)?;
    let second_vector = second
        .strength
        .sub(second.point)
        .map_err(LiquifyExecutionError::Validation)?;
    let first_angle = first_vector.y().atan2(first_vector.x());
    let second_angle = second_vector.y().atan2(second_vector.x());
    let mut delta = second_angle - first_angle;
    if delta > std::f32::consts::PI {
        delta -= 2.0 * std::f32::consts::PI;
    }
    if delta < -std::f32::consts::PI {
        delta += 2.0 * std::f32::consts::PI;
    }
    let magnitude = lerp(
        first_vector.distance(LiquifyPoint::zero()),
        second_vector.distance(LiquifyPoint::zero()),
        t,
    );
    let angle = first_angle + delta * t;
    let strength = center
        .add(
            LiquifyPoint::new(angle.cos() * magnitude, angle.sin() * magnitude)
                .map_err(LiquifyExecutionError::Validation)?,
        )
        .map_err(LiquifyExecutionError::Validation)?;
    Ok(Stamp {
        center,
        strength,
        radius: lerp(
            first.radius.distance(first.point),
            second.radius.distance(second.point),
            t,
        ),
        control1: lerp(first.control1(), second.control1(), t),
        control2: lerp(first.control2(), second.control2(), t),
        warp_type: first.warp_type,
        relocated: true,
    })
}

pub(super) fn lerp(first: f32, second: f32, t: f32) -> f32 {
    first + (second - first) * t
}

fn cubic_samples(
    p0: LiquifyPoint,
    p1: LiquifyPoint,
    p2: LiquifyPoint,
    p3: LiquifyPoint,
) -> Vec<LiquifyPoint> {
    (0..LIQUIFY_INTERPOLATION_POINTS)
        .map(|index| {
            let t = index as f32 / (LIQUIFY_INTERPOLATION_POINTS - 1) as f32;
            let u = 1.0 - t;
            LiquifyPoint::new(
                u * u * u * p0.x()
                    + 3.0 * u * u * t * p1.x()
                    + 3.0 * u * t * t * p2.x()
                    + t * t * t * p3.x(),
                u * u * u * p0.y()
                    + 3.0 * u * u * t * p1.y()
                    + 3.0 * u * t * t * p2.y()
                    + t * t * t * p3.y(),
            )
            .expect("validated Bezier points remain finite")
        })
        .collect()
}

fn point_at_length(samples: &[LiquifyPoint], distance: f32) -> LiquifyPoint {
    let mut accumulated = 0.0;
    for pair in samples.windows(2) {
        let segment = pair[0].distance(pair[1]);
        if accumulated + segment >= distance {
            let t = if segment > 0.0 {
                (distance - accumulated) / segment
            } else {
                0.0
            };
            return LiquifyPoint::new(
                lerp(pair[0].x(), pair[1].x(), t),
                lerp(pair[0].y(), pair[1].y(), t),
            )
            .expect("finite curve sample");
        }
        accumulated += segment;
    }
    samples.last().copied().unwrap_or(LiquifyPoint::zero())
}

pub(super) fn inverse_point(
    target: LiquifyPoint,
    stamps: &[Stamp],
) -> Result<LiquifyPoint, LiquifyExecutionError> {
    let mut source = target;
    for _ in 0..INVERSE_ITERATIONS {
        let mapped = forward_point(source, stamps)?;
        let delta = target
            .sub(mapped)
            .map_err(LiquifyExecutionError::Validation)?;
        source = source
            .add(delta)
            .map_err(LiquifyExecutionError::Validation)?;
        if delta.x().hypot(delta.y()) <= INVERSE_TOLERANCE {
            return Ok(source);
        }
    }
    Err(LiquifyExecutionError::NonConvergentInverse {
        x: target.x(),
        y: target.y(),
    })
}

pub(super) fn forward_point(
    mut point: LiquifyPoint,
    stamps: &[Stamp],
) -> Result<LiquifyPoint, LiquifyExecutionError> {
    for stamp in stamps {
        let delta = point
            .sub(stamp.center)
            .map_err(LiquifyExecutionError::Validation)?;
        let distance = delta.x().hypot(delta.y());
        if distance >= stamp.radius {
            continue;
        }
        let influence = falloff(stamp.control1, stamp.control2, distance / stamp.radius);
        let factor = if stamp.relocated {
            LIQUIFY_STAMP_RELOCATION
        } else {
            1.0
        };
        let vector = stamp
            .strength
            .sub(stamp.center)
            .map_err(LiquifyExecutionError::Validation)?
            .scale(0.5 * influence * factor)
            .map_err(LiquifyExecutionError::Validation)?;
        let displacement = match stamp.warp_type {
            LiquifyWarpType::Linear => vector
                .scale(-1.0)
                .map_err(LiquifyExecutionError::Validation)?,
            LiquifyWarpType::RadialGrow | LiquifyWarpType::RadialShrink => {
                let sign = if stamp.warp_type == LiquifyWarpType::RadialShrink {
                    -1.0
                } else {
                    1.0
                };
                delta
                    .scale(
                        -sign * 0.5 * vector.distance(LiquifyPoint::zero()) / stamp.radius
                            * influence
                            * factor,
                    )
                    .map_err(LiquifyExecutionError::Validation)?
            }
        };
        point = point
            .add(displacement)
            .map_err(LiquifyExecutionError::Validation)?;
    }
    Ok(point)
}

pub(super) fn falloff(control1: f32, control2: f32, normalized_distance: f32) -> f32 {
    if normalized_distance <= 0.0 {
        return 1.0;
    }
    if normalized_distance >= 1.0 {
        return 0.0;
    }
    let mut low = 0.0;
    let mut high = 1.0;
    for _ in 0..16 {
        let t = f32::midpoint(low, high);
        let x = 3.0 * (1.0 - t) * (1.0 - t) * t * control1
            + 3.0 * (1.0 - t) * t * t * control2
            + t * t * t;
        if x < normalized_distance {
            low = t;
        } else {
            high = t;
        }
    }
    let t = f32::midpoint(low, high);
    let u = 1.0 - t;
    u * u * u + 3.0 * u * u * t
}

pub(super) fn sample_plane(
    input: &[f32],
    dimensions: RasterDimensions,
    point: LiquifyPoint,
) -> f32 {
    let x = point
        .x()
        .clamp(0.0, dimensions.width().saturating_sub(1) as f32);
    let y = point
        .y()
        .clamp(0.0, dimensions.height().saturating_sub(1) as f32);
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(dimensions.width().saturating_sub(1));
    let y1 = (y0 + 1).min(dimensions.height().saturating_sub(1));
    let tx = x - x0 as f32;
    let ty = y - y0 as f32;
    let index = |xx: u32, yy: u32| {
        usize::try_from(yy * dimensions.width() + xx).expect("bounded image index")
    };
    let top = lerp(input[index(x0, y0)], input[index(x1, y0)], tx);
    let bottom = lerp(input[index(x0, y1)], input[index(x1, y1)], tx);
    lerp(top, bottom, ty)
}

pub(super) fn sample_channel(
    input: &[LinearRgb],
    dimensions: RasterDimensions,
    point: LiquifyPoint,
    interpolation: LiquifyInterpolation,
    channel: usize,
) -> Result<f32, LiquifyExecutionError> {
    if interpolation == LiquifyInterpolation::Bilinear {
        let plane: Vec<f32> = input
            .iter()
            .map(|pixel| match channel {
                0 => pixel.red().get(),
                1 => pixel.green().get(),
                _ => pixel.blue().get(),
            })
            .collect();
        return Ok(sample_plane(&plane, dimensions, point));
    }
    let radius = match interpolation {
        LiquifyInterpolation::Bicubic | LiquifyInterpolation::Lanczos2 => 2,
        LiquifyInterpolation::Lanczos3 => 3,
        LiquifyInterpolation::Bilinear => 1,
    };
    let x = point
        .x()
        .clamp(0.0, dimensions.width().saturating_sub(1) as f32);
    let y = point
        .y()
        .clamp(0.0, dimensions.height().saturating_sub(1) as f32);
    let center_x = x.floor() as i32;
    let center_y = y.floor() as i32;
    let mut value = 0.0;
    let mut weight_sum = 0.0;
    for yy in center_y + 1 - radius..=center_y + radius {
        for xx in center_x + 1 - radius..=center_x + radius {
            let wx = kernel_weight(interpolation, x - xx as f32);
            let wy = kernel_weight(interpolation, y - yy as f32);
            let weight = wx * wy;
            let width = i32::try_from(dimensions.width().saturating_sub(1)).unwrap_or(i32::MAX);
            let height = i32::try_from(dimensions.height().saturating_sub(1)).unwrap_or(i32::MAX);
            let sx = xx.clamp(0, width) as u32;
            let sy = yy.clamp(0, height) as u32;
            let index = usize::try_from(sy * dimensions.width() + sx)
                .map_err(|_| LiquifyExecutionError::ArithmeticOverflow)?;
            let sample = match channel {
                0 => input[index].red().get(),
                1 => input[index].green().get(),
                _ => input[index].blue().get(),
            };
            value += sample * weight;
            weight_sum += weight;
        }
    }
    let value = value / weight_sum.max(f32::EPSILON);
    if value.is_finite() {
        Ok(value)
    } else {
        Err(LiquifyExecutionError::SampleNonFinite)
    }
}

fn kernel_weight(interpolation: LiquifyInterpolation, x: f32) -> f32 {
    let ax = x.abs();
    match interpolation {
        LiquifyInterpolation::Bicubic => {
            if ax <= 1.0 {
                1.5 * ax.powi(3) - 2.5 * ax.powi(2) + 1.0
            } else if ax < 2.0 {
                -0.5 * ax.powi(3) + 2.5 * ax.powi(2) - 4.0 * ax + 2.0
            } else {
                0.0
            }
        }
        LiquifyInterpolation::Lanczos2 | LiquifyInterpolation::Lanczos3 => {
            let radius = if interpolation == LiquifyInterpolation::Lanczos2 {
                2.0
            } else {
                3.0
            };
            if ax >= radius {
                0.0
            } else if ax <= f32::EPSILON {
                1.0
            } else {
                sinc(ax) * sinc(ax / radius)
            }
        }
        LiquifyInterpolation::Bilinear => (1.0 - ax).max(0.0),
    }
}

fn sinc(x: f32) -> f32 {
    (std::f32::consts::PI * x).sin() / (std::f32::consts::PI * x)
}

pub(super) fn sample_rgb(
    input: &[LinearRgb],
    dimensions: RasterDimensions,
    point: LiquifyPoint,
    interpolation: LiquifyInterpolation,
) -> Result<LinearRgb, LiquifyExecutionError> {
    let values = [
        sample_channel(input, dimensions, point, interpolation, 0)?,
        sample_channel(input, dimensions, point, interpolation, 1)?,
        sample_channel(input, dimensions, point, interpolation, 2)?,
    ];
    Ok(LinearRgb::new(
        FiniteF32::new(values[0]).map_err(|_| LiquifyExecutionError::SampleNonFinite)?,
        FiniteF32::new(values[1]).map_err(|_| LiquifyExecutionError::SampleNonFinite)?,
        FiniteF32::new(values[2]).map_err(|_| LiquifyExecutionError::SampleNonFinite)?,
    ))
}

pub(super) fn plan_identity(
    config: &LiquifyConfig,
    dimensions: RasterDimensions,
    interpolation: LiquifyInterpolation,
    field: &[LiquifyPoint],
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.liquify.plan.v1");
    hasher.update(config.to_bytes().expect("validated liquify config"));
    hasher.update(dimensions.width().to_le_bytes());
    hasher.update(dimensions.height().to_le_bytes());
    hasher.update([interpolation as u8]);
    for point in field {
        hasher.update(point.x().to_bits().to_le_bytes());
        hasher.update(point.y().to_bits().to_le_bytes());
    }
    hasher.finalize().into()
}
