//! Lab bilateral-grid color reconstruction ported from
//! `src/iop/colorreconstruction.c` and its coupled
//! `data/kernels/colorreconstruction.cl` equations.
//!
//! `LinearRgb` is the pixelpipe's three-channel storage carrier at this boundary;
//! its channels are interpreted here as Darktable's `(L, a, b)` values. Tiled
//! execution and preview-grid freeze/thaw are intentionally not exposed: the
//! native module requires full-image evidence and explicitly disables tiling.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::missing_errors_doc,
    clippy::must_use_candidate
)]

use std::f32::consts::PI;
use std::fmt;
use std::mem::size_of;

use crate::{FiniteF32, LinearRgb, RasterDimensions, RgbChannel};

use super::common::{
    OperationExecutionError, ReconstructionBudget, ReconstructionDiagnostics,
    ReconstructionReceipt, validate_shape,
};

pub const COLORRECONSTRUCTION_COMPATIBILITY_ID: &str = "colorreconstruct";
pub const COLORRECONSTRUCTION_SCHEMA_VERSION: u16 = 3;
pub const BILATERAL_MAX_RESOLUTION_SPATIAL: usize = 500;
pub const BILATERAL_MAX_RESOLUTION_RANGE: usize = 100;
pub const SPATIAL_PREVIEW_APPROXIMATION_THRESHOLD: f32 = 100.0;
pub const COLORRECONSTRUCTION_DEFAULT_HUE: f32 = 0.66;
pub const COLORRECONSTRUCTION_MINIMUM_SPATIAL_SIGMA: f32 = 1.0;
pub const COLORRECONSTRUCTION_MINIMUM_RANGE_SIGMA: f32 = 0.1;
pub const COLORRECONSTRUCTION_LAB_LIGHTNESS_RANGE: f32 = 100.0;
const DEFAULT_HUE: f32 = COLORRECONSTRUCTION_DEFAULT_HUE;
const MINIMUM_SPATIAL_SIGMA: f32 = COLORRECONSTRUCTION_MINIMUM_SPATIAL_SIGMA;
const MINIMUM_RANGE_SIGMA: f32 = COLORRECONSTRUCTION_MINIMUM_RANGE_SIGMA;
const LAB_LIGHTNESS_RANGE: f32 = COLORRECONSTRUCTION_LAB_LIGHTNESS_RANGE;
const HUE_VARIANCE: f32 = PI * PI / 8.0;
const BLUR_W0: f32 = 6.0 / 16.0;
const BLUR_W1: f32 = 4.0 / 16.0;
const BLUR_W2: f32 = 1.0 / 16.0;

/// `dt_iop_colorreconstruct_precedence_t`, retained numerically for imports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorReconstructionPrecedence {
    None,
    Chroma,
    Hue,
}

impl ColorReconstructionPrecedence {
    #[must_use]
    pub const fn id(self) -> i32 {
        match self {
            Self::None => 0,
            Self::Chroma => 1,
            Self::Hue => 2,
        }
    }

    pub fn from_id(id: i32) -> Result<Self, ColorReconstructionParameterError> {
        match id {
            0 => Ok(Self::None),
            1 => Ok(Self::Chroma),
            2 => Ok(Self::Hue),
            _ => Err(ColorReconstructionParameterError::UnknownPrecedence(id)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColorReconstructionConfig {
    threshold: FiniteF32,
    spatial: FiniteF32,
    range: FiniteF32,
    hue: FiniteF32,
    precedence: ColorReconstructionPrecedence,
}

impl ColorReconstructionConfig {
    pub fn new(
        threshold: f32,
        spatial: f32,
        range: f32,
        hue: f32,
        precedence: ColorReconstructionPrecedence,
    ) -> Result<Self, ColorReconstructionParameterError> {
        Ok(Self {
            // Darktable's ranges are editor presentation bounds. `commit_params`
            // copies every finite persisted value into the processing state.
            threshold: finite("threshold", threshold)?,
            spatial: finite("spatial", spatial)?,
            range: finite("range", range)?,
            hue: finite("hue", hue)?,
            precedence,
        })
    }

    #[must_use]
    pub const fn threshold(self) -> FiniteF32 {
        self.threshold
    }

    #[must_use]
    pub const fn spatial(self) -> FiniteF32 {
        self.spatial
    }

    #[must_use]
    pub const fn range(self) -> FiniteF32 {
        self.range
    }

    #[must_use]
    pub const fn hue(self) -> FiniteF32 {
        self.hue
    }

    #[must_use]
    pub const fn precedence(self) -> ColorReconstructionPrecedence {
        self.precedence
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorReconstructionParameterError {
    UnknownPrecedence(i32),
    OutOfRange { name: &'static str, value: u32 },
    NonFinite(&'static str),
}

impl fmt::Display for ColorReconstructionParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownPrecedence(id) => {
                write!(formatter, "unknown color reconstruction precedence {id}")
            }
            Self::OutOfRange { name, value } => {
                write!(formatter, "{name} is out of range ({value})")
            }
            Self::NonFinite(name) => write!(formatter, "{name} is non-finite"),
        }
    }
}

impl std::error::Error for ColorReconstructionParameterError {}

fn finite(name: &'static str, value: f32) -> Result<FiniteF32, ColorReconstructionParameterError> {
    FiniteF32::new(value).map_err(|_| ColorReconstructionParameterError::NonFinite(name))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorReconstructionV1 {
    pub threshold: f32,
    pub spatial: f32,
    pub range: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorReconstructionV2 {
    pub threshold: f32,
    pub spatial: f32,
    pub range: f32,
    pub precedence: i32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorReconstructionV3 {
    pub threshold: f32,
    pub spatial: f32,
    pub range: f32,
    pub hue: f32,
    pub precedence: i32,
}

/// Native v1 -> v3 migration: add neutral precedence and the historical hue.
#[must_use]
pub fn migrate_v1(value: ColorReconstructionV1) -> ColorReconstructionV3 {
    ColorReconstructionV3 {
        threshold: value.threshold,
        spatial: value.spatial,
        range: value.range,
        hue: DEFAULT_HUE,
        precedence: ColorReconstructionPrecedence::None.id(),
    }
}

/// Native v2 -> v3 migration: preserve precedence and add the historical hue.
#[must_use]
pub fn migrate_v2(value: ColorReconstructionV2) -> ColorReconstructionV3 {
    ColorReconstructionV3 {
        threshold: value.threshold,
        spatial: value.spatial,
        range: value.range,
        hue: DEFAULT_HUE,
        precedence: value.precedence,
    }
}

impl ColorReconstructionV3 {
    pub fn config(self) -> Result<ColorReconstructionConfig, ColorReconstructionParameterError> {
        ColorReconstructionConfig::new(
            self.threshold,
            self.spatial,
            self.range,
            self.hue,
            ColorReconstructionPrecedence::from_id(self.precedence)?,
        )
    }
}

/// Full-frame bilateral-grid geometry produced by the native init routine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorReconstructionGridGeometry {
    size_x: usize,
    size_y: usize,
    size_z: usize,
    sigma_s: f32,
    sigma_r: f32,
}

impl ColorReconstructionGridGeometry {
    fn new(dimensions: RasterDimensions, config: ColorReconstructionConfig) -> Self {
        // This plan is the full-image, unit-scale CPU boundary. The native CPU
        // path clamps `iscale / roi.scale` to at least one before deriving sigma.
        let requested_sigma_s = config.spatial().get().max(MINIMUM_SPATIAL_SIGMA);
        let requested_sigma_r = config.range().get().max(MINIMUM_RANGE_SIGMA);
        let size_x = grid_extent(
            dimensions.width() as f32,
            requested_sigma_s,
            BILATERAL_MAX_RESOLUTION_SPATIAL,
        );
        let size_y = grid_extent(
            dimensions.height() as f32,
            requested_sigma_s,
            BILATERAL_MAX_RESOLUTION_SPATIAL,
        );
        let size_z = grid_extent(
            LAB_LIGHTNESS_RANGE,
            requested_sigma_r,
            BILATERAL_MAX_RESOLUTION_RANGE,
        );
        let sigma_s = (dimensions.height() as f32 / (size_y - 1) as f32)
            .max(dimensions.width() as f32 / (size_x - 1) as f32);
        let sigma_r = LAB_LIGHTNESS_RANGE / (size_z - 1) as f32;
        Self {
            size_x,
            size_y,
            size_z,
            sigma_s,
            sigma_r,
        }
    }

    #[must_use]
    pub const fn size_x(self) -> usize {
        self.size_x
    }

    #[must_use]
    pub const fn size_y(self) -> usize {
        self.size_y
    }

    #[must_use]
    pub const fn size_z(self) -> usize {
        self.size_z
    }

    #[must_use]
    pub const fn sigma_s(self) -> f32 {
        self.sigma_s
    }

    #[must_use]
    pub const fn sigma_r(self) -> f32 {
        self.sigma_r
    }

    fn cells(self) -> Option<usize> {
        self.size_x
            .checked_mul(self.size_y)
            .and_then(|value| value.checked_mul(self.size_z))
    }

    /// One native CPU grid buffer (`bilateral_singlebuffer_size`).
    #[must_use]
    pub fn single_buffer_bytes(self) -> usize {
        self.cells()
            .and_then(|cells| cells.checked_mul(size_of::<GridLab>()))
            .unwrap_or(usize::MAX)
    }

    /// Native tiling estimate, which reserves two grids for the `OpenCL` path.
    #[must_use]
    pub fn native_memory_estimate_bytes(self) -> usize {
        self.single_buffer_bytes().saturating_mul(2)
    }
}

fn grid_extent(span: f32, requested_sigma: f32, maximum: usize) -> usize {
    let rounded = (span / requested_sigma).round() as usize;
    rounded.clamp(4, maximum) + 1
}

/// Map a pixel in the current ROI back into the bilateral grid's image
/// coordinates. The production plan is a full-frame boundary, so its ROI and
/// bilateral origins are both zero and its rescale is one. Keeping the native
/// transform explicit prevents a future tiled caller from silently treating a
/// tile-local coordinate as a full-image coordinate.
#[must_use]
pub fn grid_rescale(
    i: i32,
    j: i32,
    roi_x: i32,
    roi_y: i32,
    bilateral_x: i32,
    bilateral_y: i32,
    scale: f32,
) -> (f32, f32) {
    (
        (roi_x + i) as f32 * scale - bilateral_x as f32,
        (roi_y + j) as f32 * scale - bilateral_y as f32,
    )
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorReconstructionPlan {
    config: ColorReconstructionConfig,
    dimensions: RasterDimensions,
    geometry: ColorReconstructionGridGeometry,
    required_bytes: usize,
}

impl ColorReconstructionPlan {
    pub fn new(
        config: ColorReconstructionConfig,
        dimensions: RasterDimensions,
        budget: ReconstructionBudget,
    ) -> Result<Self, OperationExecutionError> {
        let geometry = ColorReconstructionGridGeometry::new(dimensions, config);
        let pixel_count = usize::try_from(dimensions.pixel_count()).map_err(|_| {
            OperationExecutionError::MemoryBudgetExceeded {
                required: usize::MAX,
                budget: budget.maximum_bytes(),
            }
        })?;
        let per_pixel = size_of::<LinearRgb>()
            .checked_mul(2)
            .and_then(|value| value.checked_add(size_of::<f32>() + 2))
            .ok_or(OperationExecutionError::MemoryBudgetExceeded {
                required: usize::MAX,
                budget: budget.maximum_bytes(),
            })?;
        let required_bytes = pixel_count
            .checked_mul(per_pixel)
            .and_then(|value| value.checked_add(geometry.single_buffer_bytes()))
            .ok_or(OperationExecutionError::MemoryBudgetExceeded {
                required: usize::MAX,
                budget: budget.maximum_bytes(),
            })?;
        if required_bytes > budget.maximum_bytes() {
            return Err(OperationExecutionError::MemoryBudgetExceeded {
                required: required_bytes,
                budget: budget.maximum_bytes(),
            });
        }
        Ok(Self {
            config,
            dimensions,
            geometry,
            required_bytes,
        })
    }

    #[must_use]
    pub const fn config(self) -> ColorReconstructionConfig {
        self.config
    }

    #[must_use]
    pub const fn dimensions(self) -> RasterDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn geometry(self) -> ColorReconstructionGridGeometry {
        self.geometry
    }

    #[must_use]
    pub const fn required_bytes(self) -> usize {
        self.required_bytes
    }

    #[must_use]
    pub const fn full_image_analysis(self) -> bool {
        true
    }

    /// The source module disables tiling because every tile would infer a
    /// different replacement color field.
    #[must_use]
    pub const fn supports_tiling(self) -> bool {
        false
    }

    /// Preview-grid freeze/thaw depends on GUI pixelpipe synchronization and is
    /// deliberately deferred to that future composition boundary.
    #[must_use]
    pub const fn reuses_preview_grid(self) -> bool {
        false
    }

    #[must_use]
    pub fn support_radius(self) -> u32 {
        (4.0 * self.geometry.sigma_s()).ceil() as u32
    }

    pub fn execute(
        &self,
        input: &[LinearRgb],
    ) -> Result<ColorReconstructionExecution, OperationExecutionError> {
        self.execute_with_cancel(input, || false)
    }

    pub fn execute_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        cancelled: F,
    ) -> Result<ColorReconstructionExecution, OperationExecutionError> {
        validate_shape(self.dimensions, input)?;
        if cancelled() {
            return Err(OperationExecutionError::Cancelled);
        }

        let mut diagnostics = allocate_diagnostics(input.len(), self.required_bytes)?;
        for (index, pixel) in input.iter().copied().enumerate() {
            if index % usize::try_from(self.dimensions.width()).unwrap_or(1) == 0 && cancelled() {
                return Err(OperationExecutionError::Cancelled);
            }
            diagnostics.affected[index] =
                highlight_blend(pixel.red().get(), self.config.threshold().get()) > 0.0;
        }

        // A frame with no pixels in the native 5% transition band or above is
        // an exact copy. Preserve that source behavior without allocating the
        // bilateral grid.
        if !diagnostics.affected.iter().any(|affected| *affected) {
            let output = try_clone_pixels(input, self.required_bytes)?;
            let receipt = ReconstructionReceipt::new(
                COLORRECONSTRUCTION_COMPATIBILITY_ID,
                COLORRECONSTRUCTION_SCHEMA_VERSION,
                input,
                &output,
                &diagnostics,
            );
            return Ok(ColorReconstructionExecution {
                pixels: output,
                diagnostics,
                receipt,
            });
        }

        let mut grid = BilateralGrid::allocate(self.geometry, self.required_bytes)?;
        grid.splat(input, self.dimensions, self.config, &cancelled)?;
        grid.blur(&cancelled)?;
        let output = grid.slice(
            input,
            self.dimensions,
            self.config.threshold().get(),
            &mut diagnostics,
            &cancelled,
            self.required_bytes,
        )?;
        let receipt = ReconstructionReceipt::new(
            COLORRECONSTRUCTION_COMPATIBILITY_ID,
            COLORRECONSTRUCTION_SCHEMA_VERSION,
            input,
            &output,
            &diagnostics,
        );
        Ok(ColorReconstructionExecution {
            pixels: output,
            diagnostics,
            receipt,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColorReconstructionExecution {
    pixels: Vec<LinearRgb>,
    diagnostics: ReconstructionDiagnostics,
    receipt: ReconstructionReceipt,
}

impl ColorReconstructionExecution {
    #[must_use]
    pub fn pixels(&self) -> &[LinearRgb] {
        &self.pixels
    }

    #[must_use]
    pub const fn diagnostics(&self) -> &ReconstructionDiagnostics {
        &self.diagnostics
    }

    #[must_use]
    pub const fn receipt(&self) -> &ReconstructionReceipt {
        &self.receipt
    }
}

#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
struct GridLab {
    lightness: f32,
    a: f32,
    b: f32,
    weight: f32,
}

struct BilateralGrid {
    geometry: ColorReconstructionGridGeometry,
    cells: Vec<GridLab>,
}

impl BilateralGrid {
    fn allocate(
        geometry: ColorReconstructionGridGeometry,
        required: usize,
    ) -> Result<Self, OperationExecutionError> {
        let count = geometry
            .cells()
            .ok_or(OperationExecutionError::AllocationFailed {
                required: usize::MAX,
            })?;
        let mut cells = Vec::new();
        cells
            .try_reserve_exact(count)
            .map_err(|_| OperationExecutionError::AllocationFailed { required })?;
        cells.resize(count, GridLab::default());
        Ok(Self { geometry, cells })
    }

    fn splat<F: Fn() -> bool>(
        &mut self,
        input: &[LinearRgb],
        dimensions: RasterDimensions,
        config: ColorReconstructionConfig,
        cancelled: &F,
    ) -> Result<(), OperationExecutionError> {
        let width = dimensions.width() as usize;
        let height = dimensions.height() as usize;
        let threshold = config.threshold().get();
        let converted_hue = hue_conversion(config.hue().get());
        for y in 0..height {
            if cancelled() {
                return Err(OperationExecutionError::Cancelled);
            }
            for x in 0..width {
                let pixel = input[y * width + x];
                let lightness = pixel.red().get();
                // The CPU source deliberately ignores only values strictly
                // above threshold; threshold itself remains evidence.
                if lightness > threshold {
                    continue;
                }
                let a = pixel.green().get();
                let b = pixel.blue().get();
                let weight = match config.precedence() {
                    ColorReconstructionPrecedence::None => 1.0,
                    ColorReconstructionPrecedence::Chroma => (a * a + b * b).sqrt(),
                    ColorReconstructionPrecedence::Hue => {
                        let mut distance = b.atan2(a) - converted_hue;
                        distance = if distance > PI {
                            distance - 2.0 * PI
                        } else if distance < -PI {
                            distance + 2.0 * PI
                        } else {
                            distance
                        };
                        (-distance * distance / HUE_VARIANCE).exp()
                    }
                };
                let (grid_x, grid_y, grid_z) = self.image_to_grid(x as f32, y as f32, lightness);
                let xi = grid_x.round() as usize;
                let yi = grid_y.round() as usize;
                let zi = grid_z.round() as usize;
                let index = self.index(xi, yi, zi);
                let cell = &mut self.cells[index];
                cell.lightness += lightness * weight;
                cell.a += a * weight;
                cell.b += b * weight;
                cell.weight += weight;
            }
        }
        Ok(())
    }

    fn blur<F: Fn() -> bool>(&mut self, cancelled: &F) -> Result<(), OperationExecutionError> {
        let x = self.geometry.size_x;
        let y = self.geometry.size_y;
        let z = self.geometry.size_z;
        blur_line(&mut self.cells, x * y, x, 1, z, y, x, cancelled)?;
        blur_line(&mut self.cells, x * y, 1, x, z, x, y, cancelled)?;
        blur_line(&mut self.cells, 1, x, x * y, x, y, z, cancelled)
    }

    fn slice<F: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        dimensions: RasterDimensions,
        threshold: f32,
        diagnostics: &mut ReconstructionDiagnostics,
        cancelled: &F,
        required: usize,
    ) -> Result<Vec<LinearRgb>, OperationExecutionError> {
        let width = dimensions.width() as usize;
        let height = dimensions.height() as usize;
        let mut output = Vec::new();
        output
            .try_reserve_exact(input.len())
            .map_err(|_| OperationExecutionError::AllocationFailed { required })?;

        for y in 0..height {
            if cancelled() {
                return Err(OperationExecutionError::Cancelled);
            }
            for x in 0..width {
                let index = y * width + x;
                let pixel = input[index];
                let lightness = pixel.red().get();
                let a = pixel.green().get();
                let b = pixel.blue().get();
                let blend = highlight_blend(lightness, threshold);
                if blend == 0.0 {
                    output.push(pixel);
                    continue;
                }
                let native_x = i32::try_from(x).expect("raster width fits native coordinate");
                let native_y = i32::try_from(y).expect("raster height fits native coordinate");
                let (px, py) = grid_rescale(native_x, native_y, 0, 0, 0, 0, 1.0);
                let (grid_x, grid_y, grid_z) = self.image_to_grid(px, py, lightness);
                let xi = (grid_x as usize).min(self.geometry.size_x - 2);
                let yi = (grid_y as usize).min(self.geometry.size_y - 2);
                let zi = (grid_z as usize).min(self.geometry.size_z - 2);
                let xf = grid_x - xi as f32;
                let yf = grid_y - yi as f32;
                let zf = grid_z - zi as f32;
                let sampled = self.trilinear(xi, yi, zi, xf, yf, zf);
                if sampled.weight > 0.0 {
                    let sampled_lightness = sampled.lightness.max(0.01);
                    let out_a =
                        a * (1.0 - blend) + sampled.a * lightness / sampled_lightness * blend;
                    let out_b =
                        b * (1.0 - blend) + sampled.b * lightness / sampled_lightness * blend;
                    let result = lab_pixel(lightness, out_a, out_b, index)?;
                    diagnostics.candidate[index] = true;
                    diagnostics.confidence[index] = sampled.weight / (sampled.weight + 1.0);
                    diagnostics.contribution[index] = difference(pixel, result, index)?;
                    output.push(result);
                } else {
                    output.push(pixel);
                }
            }
        }
        Ok(output)
    }

    fn image_to_grid(&self, x: f32, y: f32, lightness: f32) -> (f32, f32, f32) {
        (
            (x / self.geometry.sigma_s).clamp(0.0, (self.geometry.size_x - 1) as f32),
            (y / self.geometry.sigma_s).clamp(0.0, (self.geometry.size_y - 1) as f32),
            (lightness / self.geometry.sigma_r).clamp(0.0, (self.geometry.size_z - 1) as f32),
        )
    }

    const fn index(&self, x: usize, y: usize, z: usize) -> usize {
        x + self.geometry.size_x * (y + self.geometry.size_y * z)
    }

    fn trilinear(&self, x: usize, y: usize, z: usize, xf: f32, yf: f32, zf: f32) -> GridLab {
        let base = self.index(x, y, z);
        let offset_x = 1;
        let offset_y = self.geometry.size_x;
        let offset_z = self.geometry.size_x * self.geometry.size_y;
        let corners = [
            (base, 1.0 - xf, 1.0 - yf, 1.0 - zf),
            (base + offset_x, xf, 1.0 - yf, 1.0 - zf),
            (base + offset_y, 1.0 - xf, yf, 1.0 - zf),
            (base + offset_x + offset_y, xf, yf, 1.0 - zf),
            (base + offset_z, 1.0 - xf, 1.0 - yf, zf),
            (base + offset_x + offset_z, xf, 1.0 - yf, zf),
            (base + offset_y + offset_z, 1.0 - xf, yf, zf),
            (base + offset_x + offset_y + offset_z, xf, yf, zf),
        ];
        let mut result = GridLab::default();
        for (index, weight_x, weight_y, weight_z) in corners {
            let cell = self.cells[index];
            result.lightness += cell.lightness * weight_x * weight_y * weight_z;
            result.a += cell.a * weight_x * weight_y * weight_z;
            result.b += cell.b * weight_x * weight_y * weight_z;
            result.weight += cell.weight * weight_x * weight_y * weight_z;
        }
        result
    }
}

// Keep the seven native stride/extent arguments adjacent for source comparison.
#[allow(clippy::too_many_arguments)]
fn blur_line<F: Fn() -> bool>(
    cells: &mut [GridLab],
    offset1: usize,
    offset2: usize,
    offset3: usize,
    size1: usize,
    size2: usize,
    size3: usize,
    cancelled: &F,
) -> Result<(), OperationExecutionError> {
    debug_assert!(size3 >= 5);
    for k in 0..size1 {
        if cancelled() {
            return Err(OperationExecutionError::Cancelled);
        }
        for j in 0..size2 {
            if cancelled() {
                return Err(OperationExecutionError::Cancelled);
            }
            let mut index = k * offset1 + j * offset2;
            let mut previous_two = cells[index];
            cells[index] = weighted3(
                cells[index],
                cells[index + offset3],
                cells[index + 2 * offset3],
            );
            index += offset3;
            let mut previous_one = cells[index];
            cells[index] = weighted4(
                cells[index],
                cells[index + offset3],
                previous_two,
                cells[index + 2 * offset3],
            );
            index += offset3;
            for _ in 2..size3 - 2 {
                let current = cells[index];
                cells[index] = weighted5(
                    cells[index],
                    cells[index + offset3],
                    previous_one,
                    cells[index + 2 * offset3],
                    previous_two,
                );
                index += offset3;
                previous_two = previous_one;
                previous_one = current;
            }
            let penultimate_original = cells[index];
            cells[index] = weighted4_edge(
                cells[index],
                cells[index + offset3],
                previous_one,
                previous_two,
            );
            index += offset3;
            cells[index] = weighted3(cells[index], penultimate_original, previous_one);
        }
    }
    Ok(())
}

fn weighted3(center: GridLab, near: GridLab, far: GridLab) -> GridLab {
    combine(center, BLUR_W0, near, BLUR_W1, far, BLUR_W2)
}

fn weighted4(center: GridLab, next: GridLab, previous: GridLab, far: GridLab) -> GridLab {
    add_scaled(
        add_scaled(scale(center, BLUR_W0), add(next, previous), BLUR_W1),
        far,
        BLUR_W2,
    )
}

fn weighted5(
    center: GridLab,
    next: GridLab,
    previous: GridLab,
    far_next: GridLab,
    far_previous: GridLab,
) -> GridLab {
    add_scaled(
        add_scaled(scale(center, BLUR_W0), add(next, previous), BLUR_W1),
        add(far_next, far_previous),
        BLUR_W2,
    )
}

fn weighted4_edge(
    center: GridLab,
    next: GridLab,
    previous: GridLab,
    far_previous: GridLab,
) -> GridLab {
    add_scaled(
        add_scaled(scale(center, BLUR_W0), add(next, previous), BLUR_W1),
        far_previous,
        BLUR_W2,
    )
}

fn add(left: GridLab, right: GridLab) -> GridLab {
    GridLab {
        lightness: left.lightness + right.lightness,
        a: left.a + right.a,
        b: left.b + right.b,
        weight: left.weight + right.weight,
    }
}

fn scale(value: GridLab, factor: f32) -> GridLab {
    GridLab {
        lightness: value.lightness * factor,
        a: value.a * factor,
        b: value.b * factor,
        weight: value.weight * factor,
    }
}

fn add_scaled(value: GridLab, addend: GridLab, factor: f32) -> GridLab {
    add(value, scale(addend, factor))
}

fn combine(
    first: GridLab,
    first_weight: f32,
    second: GridLab,
    second_weight: f32,
    third: GridLab,
    third_weight: f32,
) -> GridLab {
    add(
        add(scale(first, first_weight), scale(second, second_weight)),
        scale(third, third_weight),
    )
}

fn allocate_diagnostics(
    pixel_count: usize,
    required: usize,
) -> Result<ReconstructionDiagnostics, OperationExecutionError> {
    let zero = lab_pixel(0.0, 0.0, 0.0, 0)?;
    let mut affected = Vec::new();
    let mut candidate = Vec::new();
    let mut confidence = Vec::new();
    let mut contribution = Vec::new();
    affected
        .try_reserve_exact(pixel_count)
        .map_err(|_| OperationExecutionError::AllocationFailed { required })?;
    candidate
        .try_reserve_exact(pixel_count)
        .map_err(|_| OperationExecutionError::AllocationFailed { required })?;
    confidence
        .try_reserve_exact(pixel_count)
        .map_err(|_| OperationExecutionError::AllocationFailed { required })?;
    contribution
        .try_reserve_exact(pixel_count)
        .map_err(|_| OperationExecutionError::AllocationFailed { required })?;
    affected.resize(pixel_count, false);
    candidate.resize(pixel_count, false);
    confidence.resize(pixel_count, 0.0);
    contribution.resize(pixel_count, zero);
    Ok(ReconstructionDiagnostics {
        affected,
        candidate,
        confidence,
        contribution,
    })
}

fn try_clone_pixels(
    input: &[LinearRgb],
    required: usize,
) -> Result<Vec<LinearRgb>, OperationExecutionError> {
    let mut output = Vec::new();
    output
        .try_reserve_exact(input.len())
        .map_err(|_| OperationExecutionError::AllocationFailed { required })?;
    output.extend_from_slice(input);
    Ok(output)
}

fn highlight_blend(lightness: f32, threshold: f32) -> f32 {
    (20.0 / threshold * lightness - 19.0).clamp(0.0, 1.0)
}

/// Convert the HSL editor hue to the `LCh` hue used for precedence weighting.
#[must_use]
pub fn hue_conversion(hsl_hue: f32) -> f32 {
    let rgb = hsl_to_rgb(hsl_hue, 1.0, 0.5);
    let xyz = [
        0.436_074_7 * rgb[0] + 0.385_064_9 * rgb[1] + 0.143_080_4 * rgb[2],
        0.222_504_5 * rgb[0] + 0.716_878_6 * rgb[1] + 0.060_616_9 * rgb[2],
        0.013_932_2 * rgb[0] + 0.097_104_5 * rgb[1] + 0.714_173_3 * rgb[2],
    ];
    let epsilon = 216.0 / 24_389.0;
    let kappa = 24_389.0 / 27.0;
    let d50 = [0.9642, 1.0, 0.8249];
    let mut f = [0.0; 3];
    for channel in 0..3 {
        let value = xyz[channel] / d50[channel];
        f[channel] = if value > epsilon {
            value.cbrt()
        } else {
            (kappa * value + 16.0) / 116.0
        };
    }
    let lab_a = 500.0 * (f[0] - f[1]);
    let lab_b = -200.0 * (f[2] - f[1]);
    lab_b.atan2(lab_a)
}

fn hsl_to_rgb(mut hue: f32, saturation: f32, lightness: f32) -> [f32; 3] {
    if saturation == 0.0 {
        return [lightness; 3];
    }
    let m2 = if lightness < 0.5 {
        lightness * (1.0 + saturation)
    } else {
        lightness + saturation - lightness * saturation
    };
    let m1 = 2.0 * lightness - m2;
    hue *= 6.0;
    [
        hue_to_rgb(m1, m2, if hue < 4.0 { hue + 2.0 } else { hue - 4.0 }),
        hue_to_rgb(m1, m2, hue),
        hue_to_rgb(m1, m2, if hue > 2.0 { hue - 2.0 } else { hue + 4.0 }),
    ]
}

fn hue_to_rgb(m1: f32, m2: f32, hue: f32) -> f32 {
    if hue < 1.0 {
        m1 + (m2 - m1) * hue
    } else if hue < 3.0 {
        m2
    } else if hue < 4.0 {
        m1 + (m2 - m1) * (4.0 - hue)
    } else {
        m1
    }
}

fn lab_pixel(
    lightness: f32,
    a: f32,
    b: f32,
    index: usize,
) -> Result<LinearRgb, OperationExecutionError> {
    Ok(LinearRgb::new(
        FiniteF32::new(lightness).map_err(|_| OperationExecutionError::NonFiniteResult {
            pixel: index,
            channel: RgbChannel::Red,
        })?,
        FiniteF32::new(a).map_err(|_| OperationExecutionError::NonFiniteResult {
            pixel: index,
            channel: RgbChannel::Green,
        })?,
        FiniteF32::new(b).map_err(|_| OperationExecutionError::NonFiniteResult {
            pixel: index,
            channel: RgbChannel::Blue,
        })?,
    ))
}

fn difference(
    source: LinearRgb,
    output: LinearRgb,
    index: usize,
) -> Result<LinearRgb, OperationExecutionError> {
    lab_pixel(
        output.red().get() - source.red().get(),
        output.green().get() - source.green().get(),
        output.blue().get() - source.blue().get(),
        index,
    )
}

/// GPU parity metadata is shared with the backend-neutral registry binding.
#[must_use]
pub const fn wgpu_passes() -> [&'static str; 4] {
    [
        "colorreconstruct.splat",
        "colorreconstruct.blur-x",
        "colorreconstruct.blur-yz",
        "colorreconstruct.slice",
    ]
}
