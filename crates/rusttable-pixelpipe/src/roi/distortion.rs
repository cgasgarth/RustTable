use super::contracts::{DistortionBinding, DistortionMapping, RoiNode, RoiPlanningError, RoiRect};
use std::fmt;

const MAX_DISTORTION_DEPTH: u8 = 16;
const MAX_DISTORTION_SAMPLES: u32 = 65_536;

#[derive(Debug, Clone, Copy)]
pub(super) struct Point {
    x: f64,
    y: f64,
}

pub(super) fn enclose(
    node: &RoiNode,
    binding: &DistortionBinding,
    roi: RoiRect,
    inverse: bool,
    bounds: RoiRect,
) -> Result<RoiRect, RoiPlanningError> {
    if roi.is_empty() {
        return Err(RoiPlanningError::EmptyRoiRejected {
            operation_id: node.operation_id,
        });
    }
    let mut enclosure = Enclosure::new();
    let corners = [
        Point {
            x: f64::from(roi.x),
            y: f64::from(roi.y),
        },
        Point {
            x: f64::from(roi.right()),
            y: f64::from(roi.y),
        },
        Point {
            x: f64::from(roi.right()),
            y: f64::from(roi.bottom()),
        },
        Point {
            x: f64::from(roi.x),
            y: f64::from(roi.bottom()),
        },
    ];
    for point in corners {
        enclosure.add(map_distortion(binding, point, inverse, node)?);
    }
    for index in 0..4 {
        subdivide(
            node,
            binding,
            corners[index],
            corners[(index + 1) % 4],
            inverse,
            0,
            &mut enclosure,
        )?;
    }
    let left = (enclosure.min_x - binding.tolerance()).floor().max(0.0);
    let top = (enclosure.min_y - binding.tolerance()).floor().max(0.0);
    let right = (enclosure.max_x + binding.tolerance()).ceil().max(left);
    let bottom = (enclosure.max_y + binding.tolerance()).ceil().max(top);
    let raw = RoiRect::new(
        u32_from_f64(left)?,
        u32_from_f64(top)?,
        u32_from_f64((right - left).max(0.0))?,
        u32_from_f64((bottom - top).max(0.0))?,
    )
    .map_err(|_| RoiPlanningError::Arithmetic {
        operation_id: node.operation_id,
    })?;
    Ok(raw.intersection(bounds))
}

#[derive(Debug, Clone, Copy)]
struct Enclosure {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
    samples: u32,
}
impl Enclosure {
    const fn new() -> Self {
        Self {
            min_x: f64::INFINITY,
            min_y: f64::INFINITY,
            max_x: f64::NEG_INFINITY,
            max_y: f64::NEG_INFINITY,
            samples: 0,
        }
    }
    fn add(&mut self, point: Point) {
        self.min_x = self.min_x.min(point.x);
        self.min_y = self.min_y.min(point.y);
        self.max_x = self.max_x.max(point.x);
        self.max_y = self.max_y.max(point.y);
        self.samples = self.samples.saturating_add(1);
    }
}

fn subdivide(
    node: &RoiNode,
    binding: &DistortionBinding,
    start: Point,
    end: Point,
    inverse: bool,
    depth: u8,
    enclosure: &mut Enclosure,
) -> Result<(), RoiPlanningError> {
    if enclosure.samples >= MAX_DISTORTION_SAMPLES {
        return Err(RoiPlanningError::DistortionLimit {
            operation_id: node.operation_id,
            samples: enclosure.samples,
        });
    }
    let midpoint = Point {
        x: f64::midpoint(start.x, end.x),
        y: f64::midpoint(start.y, end.y),
    };
    let mapped_start = map_distortion(binding, start, inverse, node)?;
    let mapped_end = map_distortion(binding, end, inverse, node)?;
    let mapped_midpoint = map_distortion(binding, midpoint, inverse, node)?;
    enclosure.add(mapped_midpoint);
    let chord_midpoint = Point {
        x: f64::midpoint(mapped_start.x, mapped_end.x),
        y: f64::midpoint(mapped_start.y, mapped_end.y),
    };
    let deviation = ((mapped_midpoint.x - chord_midpoint.x).powi(2)
        + (mapped_midpoint.y - chord_midpoint.y).powi(2))
    .sqrt();
    if deviation <= binding.tolerance() {
        return Ok(());
    }
    if depth >= MAX_DISTORTION_DEPTH {
        return Err(RoiPlanningError::DistortionLimit {
            operation_id: node.operation_id,
            samples: enclosure.samples,
        });
    }
    subdivide(
        node,
        binding,
        start,
        midpoint,
        inverse,
        depth + 1,
        enclosure,
    )?;
    subdivide(node, binding, midpoint, end, inverse, depth + 1, enclosure)
}

fn map_distortion(
    binding: &DistortionBinding,
    point: Point,
    inverse: bool,
    node: &RoiNode,
) -> Result<Point, RoiPlanningError> {
    let result = if inverse {
        binding.inverse(point)
    } else {
        binding.forward(point)
    };
    result.map_err(|error| match error {
        DistortionError::NonFinite => RoiPlanningError::NonFiniteTransform {
            operation_id: node.operation_id,
        },
        DistortionError::Singular | DistortionError::NotConverged => {
            RoiPlanningError::SingularTransform {
                operation_id: node.operation_id,
            }
        }
        DistortionError::InvalidTolerance => RoiPlanningError::InvalidContract {
            operation_id: node.operation_id,
            reason: error.to_string(),
        },
    })
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn u32_from_f64(value: f64) -> Result<u32, RoiPlanningError> {
    if !value.is_finite() || value < 0.0 || value > f64::from(u32::MAX) {
        return Err(RoiPlanningError::Arithmetic { operation_id: 0 });
    }
    Ok(value as u32)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistortionError {
    InvalidTolerance,
    NonFinite,
    Singular,
    NotConverged,
}
impl fmt::Display for DistortionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidTolerance => "distortion tolerance is invalid",
            Self::NonFinite => "distortion produced a non-finite point",
            Self::Singular => "distortion mapping is singular",
            Self::NotConverged => "distortion inverse did not converge",
        })
    }
}
impl std::error::Error for DistortionError {}

pub(super) fn validate_mapping(mapping: &DistortionMapping) -> Result<(), DistortionError> {
    let finite = match mapping {
        DistortionMapping::Affine { matrix } => matrix.iter().all(|value| value.is_finite()),
        DistortionMapping::Homography { matrix } => matrix.iter().all(|value| value.is_finite()),
        DistortionMapping::Radial {
            center_x,
            center_y,
            k1,
            k2,
        } => [*center_x, *center_y, *k1, *k2]
            .iter()
            .all(|value| value.is_finite()),
    };
    finite.then_some(()).ok_or(DistortionError::NonFinite)
}

pub(super) fn map_point(
    mapping: &DistortionMapping,
    point: Point,
    inverse: bool,
) -> Result<Point, DistortionError> {
    let point = match mapping {
        DistortionMapping::Affine { matrix } => Point {
            x: matrix[0] * point.x + matrix[1] * point.y + matrix[2],
            y: matrix[3] * point.x + matrix[4] * point.y + matrix[5],
        },
        DistortionMapping::Homography { matrix } => {
            let denominator = matrix[6] * point.x + matrix[7] * point.y + matrix[8];
            if denominator.abs() < 1e-12 {
                return Err(DistortionError::Singular);
            }
            Point {
                x: (matrix[0] * point.x + matrix[1] * point.y + matrix[2]) / denominator,
                y: (matrix[3] * point.x + matrix[4] * point.y + matrix[5]) / denominator,
            }
        }
        DistortionMapping::Radial {
            center_x,
            center_y,
            k1,
            k2,
        } if inverse => invert_radial(point, *center_x, *center_y, *k1, *k2)?,
        DistortionMapping::Radial {
            center_x,
            center_y,
            k1,
            k2,
        } => {
            let dx = point.x - center_x;
            let dy = point.y - center_y;
            let r2 = dx * dx + dy * dy;
            let factor = 1.0 + k1 * r2 + k2 * r2 * r2;
            Point {
                x: center_x + dx * factor,
                y: center_y + dy * factor,
            }
        }
    };
    if point.x.is_finite() && point.y.is_finite() {
        Ok(point)
    } else {
        Err(DistortionError::NonFinite)
    }
}

fn invert_radial(
    point: Point,
    cx: f64,
    cy: f64,
    k1: f64,
    k2: f64,
) -> Result<Point, DistortionError> {
    let mut x = point.x;
    let mut y = point.y;
    for _ in 0..32 {
        let dx = x - cx;
        let dy = y - cy;
        let r2 = dx * dx + dy * dy;
        let factor = 1.0 + k1 * r2 + k2 * r2 * r2;
        if factor.abs() < 1e-12 {
            return Err(DistortionError::Singular);
        }
        let next = Point {
            x: cx + (point.x - cx) / factor,
            y: cy + (point.y - cy) / factor,
        };
        if (next.x - x).abs() + (next.y - y).abs() < 1e-10 {
            return Ok(next);
        }
        x = next.x;
        y = next.y;
    }
    Err(DistortionError::NotConverged)
}

pub(super) fn invert_affine(matrix: [f64; 6]) -> Result<[f64; 6], DistortionError> {
    let determinant = matrix[0] * matrix[4] - matrix[1] * matrix[3];
    if !determinant.is_finite() || determinant.abs() < 1e-12 {
        return Err(DistortionError::Singular);
    }
    let inverse = [
        matrix[4] / determinant,
        -matrix[1] / determinant,
        (matrix[1] * matrix[5] - matrix[4] * matrix[2]) / determinant,
        -matrix[3] / determinant,
        matrix[0] / determinant,
        (matrix[3] * matrix[2] - matrix[0] * matrix[5]) / determinant,
    ];
    validate_mapping(&DistortionMapping::Affine { matrix: inverse })?;
    Ok(inverse)
}

pub(super) fn invert_homography(matrix: [f64; 9]) -> Result<[f64; 9], DistortionError> {
    let a = matrix;
    let cofactors = [
        a[4] * a[8] - a[5] * a[7],
        a[2] * a[7] - a[1] * a[8],
        a[1] * a[5] - a[2] * a[4],
        a[5] * a[6] - a[3] * a[8],
        a[0] * a[8] - a[2] * a[6],
        a[2] * a[3] - a[0] * a[5],
        a[3] * a[7] - a[4] * a[6],
        a[1] * a[6] - a[0] * a[7],
        a[0] * a[4] - a[1] * a[3],
    ];
    let determinant = a[0] * cofactors[0] + a[1] * cofactors[3] + a[2] * cofactors[6];
    if !determinant.is_finite() || determinant.abs() < 1e-12 {
        return Err(DistortionError::Singular);
    }
    let inverse = cofactors.map(|value| value / determinant);
    validate_mapping(&DistortionMapping::Homography { matrix: inverse })?;
    Ok(inverse)
}
