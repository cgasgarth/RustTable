use crate::RasterDimensions;
use rusttable_image::Roi;
use sha2::{Digest, Sha256};
use std::fmt;

use super::{BordersAspect, BordersBasis, BordersConfig, BordersOrientation};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BordersFrame {
    left: u32,
    right: u32,
    top: u32,
    bottom: u32,
    frame_left: u32,
    frame_right: u32,
    frame_top: u32,
    frame_bottom: u32,
    inner_left: u32,
    inner_right: u32,
    inner_top: u32,
    inner_bottom: u32,
}

impl BordersFrame {
    pub const fn left(self) -> u32 {
        self.left
    }
    pub const fn right(self) -> u32 {
        self.right
    }
    pub const fn top(self) -> u32 {
        self.top
    }
    pub const fn bottom(self) -> u32 {
        self.bottom
    }
    pub const fn frame_left(self) -> u32 {
        self.frame_left
    }
    pub const fn frame_right(self) -> u32 {
        self.frame_right
    }
    pub const fn frame_top(self) -> u32 {
        self.frame_top
    }
    pub const fn frame_bottom(self) -> u32 {
        self.frame_bottom
    }
    pub const fn inner_left(self) -> u32 {
        self.inner_left
    }
    pub const fn inner_right(self) -> u32 {
        self.inner_right
    }
    pub const fn inner_top(self) -> u32 {
        self.inner_top
    }
    pub const fn inner_bottom(self) -> u32 {
        self.inner_bottom
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BordersGeometry {
    output: RasterDimensions,
    source: Roi,
    frame: BordersFrame,
}

impl BordersGeometry {
    pub const fn output_dimensions(self) -> RasterDimensions {
        self.output
    }
    pub const fn source_rect(self) -> Roi {
        self.source
    }
    pub const fn frame(self) -> BordersFrame {
        self.frame
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BordersPlan {
    config: BordersConfig,
    source_dimensions: RasterDimensions,
    geometry: BordersGeometry,
    identity: [u8; 32],
}

impl BordersPlan {
    pub fn new(
        config: BordersConfig,
        source_dimensions: RasterDimensions,
    ) -> Result<Self, BordersPlanError> {
        let geometry = resolve_geometry(config, source_dimensions)?;
        let mut hasher = Sha256::new();
        hasher.update(b"rusttable.borders.plan.v1");
        hasher.update(config_bytes(config));
        hasher.update(source_dimensions.width().to_le_bytes());
        hasher.update(source_dimensions.height().to_le_bytes());
        hasher.update(geometry.output.width().to_le_bytes());
        hasher.update(geometry.output.height().to_le_bytes());
        Ok(Self {
            config,
            source_dimensions,
            geometry,
            identity: hasher.finalize().into(),
        })
    }

    pub const fn config(&self) -> BordersConfig {
        self.config
    }
    pub const fn source_dimensions(&self) -> RasterDimensions {
        self.source_dimensions
    }
    pub const fn geometry(&self) -> BordersGeometry {
        self.geometry
    }
    pub const fn output_dimensions(&self) -> RasterDimensions {
        self.geometry.output
    }
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }

    pub fn modify_roi_out(&self, input: Roi) -> Result<Roi, BordersPlanError> {
        ensure_roi(input, self.source_dimensions)?;
        Roi::new(
            input
                .x()
                .checked_add(self.geometry.source.x())
                .ok_or(BordersPlanError::ArithmeticOverflow)?,
            input
                .y()
                .checked_add(self.geometry.source.y())
                .ok_or(BordersPlanError::ArithmeticOverflow)?,
            input.width(),
            input.height(),
        )
        .map_err(|_| BordersPlanError::ArithmeticOverflow)
    }

    pub fn modify_roi_in(&self, output: Roi) -> Result<Option<Roi>, BordersPlanError> {
        ensure_roi(output, self.geometry.output)?;
        let source = self.geometry.source;
        let x0 = i64::from(output.x()).max(i64::from(source.x()));
        let y0 = i64::from(output.y()).max(i64::from(source.y()));
        let x1 = (i64::from(output.x()) + i64::from(output.width()))
            .min(i64::from(source.x() + source.width()));
        let y1 = (i64::from(output.y()) + i64::from(output.height()))
            .min(i64::from(source.y() + source.height()));
        if x0 >= x1 || y0 >= y1 {
            return Ok(None);
        }
        Roi::new(
            u32::try_from(x0 - i64::from(source.x()))
                .map_err(|_| BordersPlanError::ArithmeticOverflow)?,
            u32::try_from(y0 - i64::from(source.y()))
                .map_err(|_| BordersPlanError::ArithmeticOverflow)?,
            u32::try_from(x1 - x0).map_err(|_| BordersPlanError::ArithmeticOverflow)?,
            u32::try_from(y1 - y0).map_err(|_| BordersPlanError::ArithmeticOverflow)?,
        )
        .map(Some)
        .map_err(|_| BordersPlanError::ArithmeticOverflow)
    }

    pub fn forward_transform(&self, points: &mut [f32]) -> Result<(), BordersPlanError> {
        transform(
            points,
            coordinate_f32(self.geometry.source.x()),
            coordinate_f32(self.geometry.source.y()),
            true,
        )
    }
    pub fn back_transform(&self, points: &mut [f32]) -> Result<(), BordersPlanError> {
        transform(
            points,
            coordinate_f32(self.geometry.source.x()),
            coordinate_f32(self.geometry.source.y()),
            false,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BordersPlanError {
    InvalidRoi,
    NonFinite,
    InvalidGeometry,
    ArithmeticOverflow,
}
impl fmt::Display for BordersPlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "borders plan error: {self:?}")
    }
}
impl std::error::Error for BordersPlanError {}

fn resolve_geometry(
    config: BordersConfig,
    dimensions: RasterDimensions,
) -> Result<BordersGeometry, BordersPlanError> {
    let width = f64::from(dimensions.width());
    let height = f64::from(dimensions.height());
    let image_aspect = width / height;
    let mut target_aspect = match config.aspect {
        BordersAspect::Image => image_aspect,
        BordersAspect::Constant => -1.0,
        _ => f64::from(
            config
                .aspect
                .ratio()
                .ok_or(BordersPlanError::InvalidGeometry)?,
        ),
    };
    if target_aspect > 0.0 {
        target_aspect = match config.orientation {
            BordersOrientation::Auto => {
                if (image_aspect < 1.0 && target_aspect > 1.0)
                    || (image_aspect > 1.0 && target_aspect < 1.0)
                {
                    1.0 / target_aspect
                } else {
                    target_aspect
                }
            }
            BordersOrientation::Landscape => target_aspect.max(1.0),
            BordersOrientation::Portrait => target_aspect.min(1.0),
        };
    }
    let basis = select_basis(config.basis, config.aspect, width, height);
    let size = f64::from(config.size.get());
    let basis_input = if basis == BordersBasis::Width {
        width
    } else {
        height
    };
    let (output_width, output_height) = if target_aspect < 0.0 {
        let output_basis = round_checked(basis_input / (1.0 - size))?;
        if basis == BordersBasis::Width {
            (output_basis, round_checked(height + output_basis - width)?)
        } else {
            (round_checked(width + output_basis - height)?, output_basis)
        }
    } else {
        let border = basis_input * (1.0 / (1.0 - size) - 1.0);
        let mut basis_output = basis_input + border;
        let mut other_output = if basis == BordersBasis::Width {
            basis_output / target_aspect
        } else {
            basis_output * target_aspect
        };
        if (basis == BordersBasis::Width && image_aspect < 1.0)
            || (basis == BordersBasis::Height && image_aspect > 1.0)
        {
            std::mem::swap(&mut basis_output, &mut other_output);
        }
        if (basis == BordersBasis::Width && image_aspect < target_aspect)
            || (basis == BordersBasis::Height && image_aspect > target_aspect)
        {
            std::mem::swap(&mut basis_output, &mut other_output);
        }
        if basis == BordersBasis::Height {
            other_output = basis_output / target_aspect;
        }
        (round_checked(basis_output)?, round_checked(other_output)?)
    };
    let output_width = to_u32(output_width.max(width))?;
    let output_height = to_u32(output_height.max(height))?;
    if output_width > 3 * dimensions.width().max(dimensions.height())
        || output_height > 3 * dimensions.width().max(dimensions.height())
    {
        return Err(BordersPlanError::InvalidGeometry);
    }
    let delta_width = output_width - dimensions.width();
    let delta_height = output_height - dimensions.height();
    let left = to_u32(round_checked(
        f64::from(delta_width) * f64::from(config.pos_h.get()),
    )?)?;
    let top = to_u32(round_checked(
        f64::from(delta_height) * f64::from(config.pos_v.get()),
    )?)?;
    let source = Roi::new(left, top, dimensions.width(), dimensions.height())
        .map_err(|_| BordersPlanError::ArithmeticOverflow)?;
    let frame = frame_geometry(
        left,
        top,
        delta_width - left,
        delta_height - top,
        dimensions,
        config,
    );
    let output = RasterDimensions::new(output_width, output_height)
        .map_err(|_| BordersPlanError::ArithmeticOverflow)?;
    Ok(BordersGeometry {
        output,
        source,
        frame,
    })
}

fn select_basis(
    basis: BordersBasis,
    aspect: BordersAspect,
    width: f64,
    height: f64,
) -> BordersBasis {
    match basis {
        BordersBasis::Auto => {
            if matches!(aspect, BordersAspect::Constant) {
                if width > height {
                    BordersBasis::Width
                } else {
                    BordersBasis::Height
                }
            } else {
                BordersBasis::Width
            }
        }
        BordersBasis::Longer => {
            if width > height {
                BordersBasis::Width
            } else {
                BordersBasis::Height
            }
        }
        BordersBasis::Shorter => {
            if width < height {
                BordersBasis::Width
            } else {
                BordersBasis::Height
            }
        }
        other => other,
    }
}

fn frame_geometry(
    left: u32,
    top: u32,
    right: u32,
    bottom: u32,
    dimensions: RasterDimensions,
    config: BordersConfig,
) -> BordersFrame {
    let minimum = left.min(right).min(top).min(bottom);
    let frame_width =
        to_u32((f64::from(minimum) * f64::from(config.frame_size.get())).round()).unwrap_or(0);
    let offset =
        to_u32((f64::from(minimum - frame_width) * f64::from(config.frame_offset.get())).floor())
            .unwrap_or(0);
    let source_right = left.saturating_add(dimensions.width());
    let source_bottom = top.saturating_add(dimensions.height());
    BordersFrame {
        left,
        right,
        top,
        bottom,
        frame_left: left.saturating_sub(offset).saturating_sub(frame_width),
        frame_right: source_right
            .saturating_add(offset)
            .saturating_add(frame_width),
        frame_top: top.saturating_sub(offset).saturating_sub(frame_width),
        frame_bottom: source_bottom
            .saturating_add(offset)
            .saturating_add(frame_width),
        inner_left: left.saturating_sub(offset),
        inner_right: source_right.saturating_add(offset),
        inner_top: top.saturating_sub(offset),
        inner_bottom: source_bottom.saturating_add(offset),
    }
}

fn config_bytes(config: BordersConfig) -> [u8; 36] {
    let mut bytes = [0; 36];
    for (index, value) in config.color.floats().into_iter().enumerate() {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes[12..16].copy_from_slice(&config.aspect.storage_value().to_le_bytes());
    for (offset, value) in [
        (16, config.size.get()),
        (20, config.pos_h.get()),
        (24, config.pos_v.get()),
        (28, config.frame_size.get()),
        (32, config.frame_offset.get()),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}
fn round_checked(value: f64) -> Result<f64, BordersPlanError> {
    if !value.is_finite() || value < 0.0 || value > f64::from(u32::MAX) {
        Err(BordersPlanError::InvalidGeometry)
    } else {
        Ok(value.round())
    }
}
fn to_u32(value: f64) -> Result<u32, BordersPlanError> {
    if !value.is_finite() || value < 0.0 || value > f64::from(u32::MAX) {
        Err(BordersPlanError::ArithmeticOverflow)
    } else {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "value is finite and bounded to u32 above"
        )]
        Ok(value as u32)
    }
}
fn ensure_roi(roi: Roi, dimensions: RasterDimensions) -> Result<(), BordersPlanError> {
    if roi
        .x()
        .checked_add(roi.width())
        .is_none_or(|right| right > dimensions.width())
        || roi
            .y()
            .checked_add(roi.height())
            .is_none_or(|bottom| bottom > dimensions.height())
    {
        Err(BordersPlanError::InvalidRoi)
    } else {
        Ok(())
    }
}
fn transform(points: &mut [f32], x: f32, y: f32, forward: bool) -> Result<(), BordersPlanError> {
    if !points.len().is_multiple_of(2) {
        return Err(BordersPlanError::InvalidRoi);
    }
    for index in (0..points.len()).step_by(2) {
        let point = &mut points[index..index + 2];
        if !point[0].is_finite() || !point[1].is_finite() {
            return Err(BordersPlanError::NonFinite);
        }
        if forward {
            point[0] += x;
            point[1] += y;
        } else {
            point[0] -= x;
            point[1] -= y;
        }
    }
    Ok(())
}

#[expect(
    clippy::cast_precision_loss,
    reason = "the public transform API is f32-based"
)]
fn coordinate_f32(value: u32) -> f32 {
    value as f32
}
