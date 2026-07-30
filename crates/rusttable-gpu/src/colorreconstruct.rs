//! Safe WGPU port of Darktable's `src/iop/colorreconstruction.c` `process_cl`
//! path and `data/kernels/colorreconstruction.cl`.

use std::fmt;
use std::num::NonZeroU64;

use crate::{CancellationToken, FaultState, GpuRuntime};

/// Retained maximum spatial bilateral-grid resolution, before the guard cell.
pub const COLORRECONSTRUCTION_MAX_SPATIAL_RESOLUTION: u32 = 500;
/// Retained maximum range bilateral-grid resolution, before the guard cell.
pub const COLORRECONSTRUCTION_MAX_RANGE_RESOLUTION: u32 = 100;
/// Spatial extent above which Darktable may reuse a preview-pipeline grid.
pub const COLORRECONSTRUCTION_SPATIAL_APPROXIMATION: f32 = 100.0;

const PARAMS_SIZE: u64 = 80;
const PARAMS_BYTES: usize = 80;
const PIXEL_BYTES: u64 = 16;
const PIXEL_BYTES_USIZE: usize = 16;
const GRID_CELL_BYTES: u64 = 16;
const DEVICE_PIXEL_BUFFER_COUNT: u64 = 3;
const HOST_PIXEL_BUFFER_COUNT: u64 = 1;
const GRID_BUFFER_COUNT: u64 = 2;
const WORKGROUP_STORAGE_BYTES: u32 = 5_120;
const ZERO_WORKGROUP: [u32; 3] = [16, 16, 1];
const SPLAT_WORKGROUP: [u32; 3] = [16, 16, 1];
const BLUR_WORKGROUP: [u32; 3] = [16, 16, 1];
const SLICE_WORKGROUP: [u32; 3] = [16, 16, 1];
const COLORRECONSTRUCTION_SHADER_SOURCE: &str = include_str!("../shaders/colorreconstruct.wgsl");

const STAGE_ORDER: [&str; 6] = [
    "colorreconstruction_zero",
    "colorreconstruction_splat",
    "colorreconstruction_blur_x",
    "colorreconstruction_blur_y",
    "colorreconstruction_blur_z",
    "colorreconstruction_slice",
];

/// Weighting precedence retained from `dt_iop_colorreconstruct_precedence_t`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ColorReconstructionPrecedence {
    None = 0,
    Chroma = 1,
    Hue = 2,
}

/// Input ROI used by the retained grid geometry and slice rescaling equations.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorReconstructionRoi {
    x: i32,
    y: i32,
    width: usize,
    height: usize,
    scale: f32,
}

impl ColorReconstructionRoi {
    #[must_use]
    pub const fn new(x: i32, y: i32, width: usize, height: usize, scale: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
            scale,
        }
    }

    #[must_use]
    pub const fn x(self) -> i32 {
        self.x
    }

    #[must_use]
    pub const fn y(self) -> i32 {
        self.y
    }

    #[must_use]
    pub const fn width(self) -> usize {
        self.width
    }

    #[must_use]
    pub const fn height(self) -> usize {
        self.height
    }

    #[must_use]
    pub const fn scale(self) -> f32 {
        self.scale
    }
}

/// One checked full-frame Color Reconstruction WGPU request.
#[derive(Debug, Clone, Copy)]
pub struct ColorReconstructionRequest<'a> {
    pixels: &'a [[f32; 4]],
    roi: ColorReconstructionRoi,
    iscale: f32,
    threshold: f32,
    spatial: f32,
    range: f32,
    hue: f32,
    precedence: ColorReconstructionPrecedence,
    transient_memory_budget_bytes: u64,
    cancellation: Option<&'a CancellationToken>,
}

impl<'a> ColorReconstructionRequest<'a> {
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        pixels: &'a [[f32; 4]],
        roi: ColorReconstructionRoi,
        iscale: f32,
        threshold: f32,
        spatial: f32,
        range: f32,
        hue: f32,
        precedence: ColorReconstructionPrecedence,
        transient_memory_budget_bytes: u64,
    ) -> Self {
        Self {
            pixels,
            roi,
            iscale,
            threshold,
            spatial,
            range,
            hue,
            precedence,
            transient_memory_budget_bytes,
            cancellation: None,
        }
    }

    /// Attaches checks before qualification, before submission, and after readback.
    #[must_use]
    pub const fn with_cancellation(mut self, cancellation: &'a CancellationToken) -> Self {
        self.cancellation = Some(cancellation);
        self
    }

    fn is_cancelled(self) -> bool {
        self.cancellation
            .is_some_and(CancellationToken::is_cancelled)
    }
}

/// Read-back Lab pixels and retained bilateral-grid execution metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorReconstructionResult {
    pixels: Vec<[f32; 4]>,
    dispatches: u32,
    grid_dimensions: [u32; 3],
    effective_sigma_s: f32,
    effective_sigma_r: f32,
}

impl ColorReconstructionResult {
    #[must_use]
    pub fn pixels(&self) -> &[[f32; 4]] {
        &self.pixels
    }

    #[must_use]
    pub const fn dispatches(&self) -> u32 {
        self.dispatches
    }

    #[must_use]
    pub const fn grid_dimensions(&self) -> [u32; 3] {
        self.grid_dimensions
    }

    #[must_use]
    pub const fn effective_sigma_s(&self) -> f32 {
        self.effective_sigma_s
    }

    #[must_use]
    pub const fn effective_sigma_r(&self) -> f32 {
        self.effective_sigma_r
    }

    #[must_use]
    pub fn into_pixels(self) -> Vec<[f32; 4]> {
        self.pixels
    }
}

/// Typed qualification or execution failure from Color Reconstruction WGPU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorReconstructionError {
    CpuOnly,
    Unhealthy,
    BufferShape { expected: usize, actual: usize },
    InvalidDimensions,
    InvalidScale,
    InvalidParameter(&'static str),
    NonFiniteInput { pixel: usize, component: usize },
    NonFiniteOutput { pixel: usize, component: usize },
    Cancelled,
    SizeOverflow,
    InvalidMemoryBudget,
    BufferLimit { required: u64, limit: u64 },
    AggregateMemoryLimit { required: u64, limit: u64 },
    InsufficientComputeLimits,
    TooManyWorkgroups,
    ShaderUnavailable,
    Upload(String),
    Poll(String),
    Readback(String),
}

impl fmt::Display for ColorReconstructionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CpuOnly => formatter.write_str("Color Reconstruction WGPU execution is CPU-only"),
            Self::Unhealthy => {
                formatter.write_str("Color Reconstruction WGPU execution is unhealthy")
            }
            Self::BufferShape { expected, actual } => write!(
                formatter,
                "Color Reconstruction WGPU expected {expected} pixels, got {actual}"
            ),
            Self::InvalidDimensions => {
                formatter.write_str("Color Reconstruction WGPU dimensions are invalid")
            }
            Self::InvalidScale => {
                formatter.write_str("Color Reconstruction WGPU scale is invalid")
            }
            Self::InvalidParameter(name) => write!(
                formatter,
                "Color Reconstruction WGPU {name} parameter is invalid"
            ),
            Self::NonFiniteInput { pixel, component } => write!(
                formatter,
                "Color Reconstruction WGPU input has a non-finite component {component} at pixel {pixel}"
            ),
            Self::NonFiniteOutput { pixel, component } => write!(
                formatter,
                "Color Reconstruction WGPU produced a non-finite component {component} at pixel {pixel}"
            ),
            Self::Cancelled => {
                formatter.write_str("Color Reconstruction WGPU execution was cancelled")
            }
            Self::SizeOverflow => formatter.write_str("Color Reconstruction WGPU size overflowed"),
            Self::InvalidMemoryBudget => formatter
                .write_str("Color Reconstruction WGPU transient-memory budget must be nonzero"),
            Self::BufferLimit { required, limit } => write!(
                formatter,
                "Color Reconstruction WGPU buffer requires {required} bytes; device limit is {limit}"
            ),
            Self::AggregateMemoryLimit { required, limit } => write!(
                formatter,
                "Color Reconstruction WGPU transient allocations require {required} bytes; transient-memory budget is {limit}"
            ),
            Self::InsufficientComputeLimits => formatter.write_str(
                "Color Reconstruction WGPU device compute, atomic, or binding limits are insufficient",
            ),
            Self::TooManyWorkgroups => formatter.write_str(
                "Color Reconstruction WGPU dispatch exceeds the device workgroup limit",
            ),
            Self::ShaderUnavailable => {
                formatter.write_str("Color Reconstruction WGPU shader is unavailable")
            }
            Self::Upload(error) => write!(formatter, "Color Reconstruction WGPU upload failed: {error}"),
            Self::Poll(error) => write!(formatter, "Color Reconstruction WGPU poll failed: {error}"),
            Self::Readback(error) => {
                write!(formatter, "Color Reconstruction WGPU readback failed: {error}")
            }
        }
    }
}

impl std::error::Error for ColorReconstructionError {}

#[derive(Debug, Clone, Copy)]
struct DispatchPlan {
    zero: [u32; 3],
    splat: [u32; 3],
    blur_x: [u32; 3],
    blur_y: [u32; 3],
    blur_z: [u32; 3],
    slice: [u32; 3],
}

impl DispatchPlan {
    const fn ordered(self) -> [[u32; 3]; 6] {
        [
            self.zero,
            self.splat,
            self.blur_x,
            self.blur_y,
            self.blur_z,
            self.slice,
        ]
    }
}

#[derive(Debug)]
struct ValidatedRequest {
    width: u32,
    height: u32,
    pixel_count: u32,
    grid_dimensions: [u32; 3],
    grid_cells: u32,
    pixel_bytes: u64,
    grid_bytes: u64,
    aggregate_transient_bytes: u64,
    effective_sigma_s: f32,
    effective_sigma_r: f32,
    rescale: f32,
    lch_hue: f32,
    dispatch: DispatchPlan,
}

impl ValidatedRequest {
    #[allow(clippy::too_many_lines)]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss
    )]
    fn new(
        request: ColorReconstructionRequest<'_>,
        limits: &wgpu::Limits,
    ) -> Result<Self, ColorReconstructionError> {
        let expected = request
            .roi
            .width
            .checked_mul(request.roi.height)
            .ok_or(ColorReconstructionError::SizeOverflow)?;
        if request.roi.width == 0
            || request.roi.height == 0
            || request.roi.width > i32::MAX as usize
            || request.roi.height > i32::MAX as usize
            || request
                .roi
                .x
                .checked_add(
                    i32::try_from(request.roi.width - 1)
                        .map_err(|_| ColorReconstructionError::SizeOverflow)?,
                )
                .is_none()
            || request
                .roi
                .y
                .checked_add(
                    i32::try_from(request.roi.height - 1)
                        .map_err(|_| ColorReconstructionError::SizeOverflow)?,
                )
                .is_none()
        {
            return Err(ColorReconstructionError::InvalidDimensions);
        }
        let pixel_count =
            u32::try_from(expected).map_err(|_| ColorReconstructionError::SizeOverflow)?;
        if request.pixels.len() != expected {
            return Err(ColorReconstructionError::BufferShape {
                expected,
                actual: request.pixels.len(),
            });
        }
        if !request.roi.scale.is_finite()
            || request.roi.scale <= 0.0
            || !request.iscale.is_finite()
            || request.iscale <= 0.0
        {
            return Err(ColorReconstructionError::InvalidScale);
        }
        validate_parameter("threshold", request.threshold, 50.0..=150.0)?;
        validate_parameter("spatial", request.spatial, 0.0..=1_000.0)?;
        validate_parameter("range", request.range, 0.0..=50.0)?;
        validate_parameter("hue", request.hue, 0.0..=1.0)?;
        if let Some((pixel, component)) = first_non_finite(request.pixels) {
            return Err(ColorReconstructionError::NonFiniteInput { pixel, component });
        }
        if request.transient_memory_budget_bytes == 0 {
            return Err(ColorReconstructionError::InvalidMemoryBudget);
        }

        let width =
            u32::try_from(request.roi.width).map_err(|_| ColorReconstructionError::SizeOverflow)?;
        let height = u32::try_from(request.roi.height)
            .map_err(|_| ColorReconstructionError::SizeOverflow)?;
        let scale = request.iscale / request.roi.scale;
        let rescale = request.iscale / (request.roi.scale * scale);
        let requested_sigma_s = request.spatial.max(1.0) / scale;
        let requested_sigma_r = request.range.max(0.1);
        if !scale.is_finite()
            || !rescale.is_finite()
            || rescale <= 0.0
            || !requested_sigma_s.is_finite()
            || requested_sigma_s <= 0.0
            || !requested_sigma_r.is_finite()
            || requested_sigma_r <= 0.0
        {
            return Err(ColorReconstructionError::InvalidScale);
        }

        let size_x = retained_grid_dimension(
            width as f32,
            requested_sigma_s,
            COLORRECONSTRUCTION_MAX_SPATIAL_RESOLUTION,
        )?;
        let size_y = retained_grid_dimension(
            height as f32,
            requested_sigma_s,
            COLORRECONSTRUCTION_MAX_SPATIAL_RESOLUTION,
        )?;
        let size_z = retained_grid_dimension(
            100.0,
            requested_sigma_r,
            COLORRECONSTRUCTION_MAX_RANGE_RESOLUTION,
        )?;
        let grid_dimensions = [size_x, size_y, size_z];
        let grid_cells = size_x
            .checked_mul(size_y)
            .and_then(|xy| xy.checked_mul(size_z))
            .ok_or(ColorReconstructionError::SizeOverflow)?;
        let effective_sigma_s =
            ((height as f32) / (size_y - 1) as f32).max((width as f32) / (size_x - 1) as f32);
        let effective_sigma_r = 100.0 / (size_z - 1) as f32;
        if !effective_sigma_s.is_finite()
            || effective_sigma_s <= 0.0
            || !effective_sigma_r.is_finite()
            || effective_sigma_r <= 0.0
        {
            return Err(ColorReconstructionError::InvalidScale);
        }

        let pixel_bytes = u64::try_from(expected)
            .map_err(|_| ColorReconstructionError::SizeOverflow)?
            .checked_mul(PIXEL_BYTES)
            .ok_or(ColorReconstructionError::SizeOverflow)?;
        let grid_bytes = u64::from(grid_cells)
            .checked_mul(GRID_CELL_BYTES)
            .ok_or(ColorReconstructionError::SizeOverflow)?;
        let aggregate_transient_bytes = transient_bytes(pixel_bytes, grid_bytes)?;
        if aggregate_transient_bytes > request.transient_memory_budget_bytes {
            return Err(ColorReconstructionError::AggregateMemoryLimit {
                required: aggregate_transient_bytes,
                limit: request.transient_memory_budget_bytes,
            });
        }

        let storage_limit = limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size);
        for required in [pixel_bytes, grid_bytes] {
            if required > storage_limit {
                return Err(ColorReconstructionError::BufferLimit {
                    required,
                    limit: storage_limit,
                });
            }
        }
        let uniform_limit = limits
            .max_uniform_buffer_binding_size
            .min(limits.max_buffer_size);
        if PARAMS_SIZE > uniform_limit {
            return Err(ColorReconstructionError::BufferLimit {
                required: PARAMS_SIZE,
                limit: uniform_limit,
            });
        }
        if limits.max_compute_invocations_per_workgroup < 256
            || limits.max_compute_workgroup_size_x < 16
            || limits.max_compute_workgroup_size_y < 16
            || limits.max_compute_workgroup_size_z < 1
            || limits.max_compute_workgroup_storage_size < WORKGROUP_STORAGE_BYTES
            || limits.max_bind_groups < 1
            || limits.max_bindings_per_bind_group < 5
            || limits.max_storage_buffers_per_shader_stage < 4
            || limits.max_uniform_buffers_per_shader_stage < 1
        {
            return Err(ColorReconstructionError::InsufficientComputeLimits);
        }

        let zero_width = size_x
            .checked_mul(4)
            .ok_or(ColorReconstructionError::SizeOverflow)?;
        let zero_height = size_y
            .checked_mul(size_z)
            .ok_or(ColorReconstructionError::SizeOverflow)?;
        let dispatch = DispatchPlan {
            zero: [
                zero_width.div_ceil(ZERO_WORKGROUP[0]),
                zero_height.div_ceil(ZERO_WORKGROUP[1]),
                1,
            ],
            splat: [
                width.div_ceil(SPLAT_WORKGROUP[0]),
                height.div_ceil(SPLAT_WORKGROUP[1]),
                1,
            ],
            blur_x: [
                size_z.div_ceil(BLUR_WORKGROUP[0]),
                size_y.div_ceil(BLUR_WORKGROUP[1]),
                1,
            ],
            blur_y: [
                size_z.div_ceil(BLUR_WORKGROUP[0]),
                size_x.div_ceil(BLUR_WORKGROUP[1]),
                1,
            ],
            blur_z: [
                size_x.div_ceil(BLUR_WORKGROUP[0]),
                size_y.div_ceil(BLUR_WORKGROUP[1]),
                1,
            ],
            slice: [
                width.div_ceil(SLICE_WORKGROUP[0]),
                height.div_ceil(SLICE_WORKGROUP[1]),
                1,
            ],
        };
        if dispatch
            .ordered()
            .into_iter()
            .flatten()
            .any(|groups| groups > limits.max_compute_workgroups_per_dimension)
        {
            return Err(ColorReconstructionError::TooManyWorkgroups);
        }

        let lch_hue = hue_conversion(request.hue);
        if !lch_hue.is_finite() {
            return Err(ColorReconstructionError::InvalidParameter("hue"));
        }
        Ok(Self {
            width,
            height,
            pixel_count,
            grid_dimensions,
            grid_cells,
            pixel_bytes,
            grid_bytes,
            aggregate_transient_bytes,
            effective_sigma_s,
            effective_sigma_r,
            rescale,
            lch_hue,
            dispatch,
        })
    }
}

/// Returns every explicit device and host allocation owned by this boundary.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
pub fn colorreconstruction_transient_memory_bytes(
    roi: ColorReconstructionRoi,
    iscale: f32,
    spatial: f32,
    range: f32,
) -> Result<u64, ColorReconstructionError> {
    if roi.width == 0
        || roi.height == 0
        || roi.width > i32::MAX as usize
        || roi.height > i32::MAX as usize
        || roi
            .x
            .checked_add(
                i32::try_from(roi.width - 1).map_err(|_| ColorReconstructionError::SizeOverflow)?,
            )
            .is_none()
        || roi
            .y
            .checked_add(
                i32::try_from(roi.height - 1)
                    .map_err(|_| ColorReconstructionError::SizeOverflow)?,
            )
            .is_none()
    {
        return Err(ColorReconstructionError::InvalidDimensions);
    }
    let pixels = roi
        .width
        .checked_mul(roi.height)
        .ok_or(ColorReconstructionError::SizeOverflow)?;
    u32::try_from(pixels).map_err(|_| ColorReconstructionError::SizeOverflow)?;
    if !roi.scale.is_finite() || roi.scale <= 0.0 || !iscale.is_finite() || iscale <= 0.0 {
        return Err(ColorReconstructionError::InvalidScale);
    }
    validate_parameter("spatial", spatial, 0.0..=1_000.0)?;
    validate_parameter("range", range, 0.0..=50.0)?;
    let scale = iscale / roi.scale;
    let sigma_s = spatial.max(1.0) / scale;
    let sigma_r = range.max(0.1);
    let size_x = retained_grid_dimension(
        roi.width as f32,
        sigma_s,
        COLORRECONSTRUCTION_MAX_SPATIAL_RESOLUTION,
    )?;
    let size_y = retained_grid_dimension(
        roi.height as f32,
        sigma_s,
        COLORRECONSTRUCTION_MAX_SPATIAL_RESOLUTION,
    )?;
    let size_z = retained_grid_dimension(100.0, sigma_r, COLORRECONSTRUCTION_MAX_RANGE_RESOLUTION)?;
    let pixel_bytes = u64::try_from(pixels)
        .map_err(|_| ColorReconstructionError::SizeOverflow)?
        .checked_mul(PIXEL_BYTES)
        .ok_or(ColorReconstructionError::SizeOverflow)?;
    let grid_bytes = u64::from(
        size_x
            .checked_mul(size_y)
            .and_then(|xy| xy.checked_mul(size_z))
            .ok_or(ColorReconstructionError::SizeOverflow)?,
    )
    .checked_mul(GRID_CELL_BYTES)
    .ok_or(ColorReconstructionError::SizeOverflow)?;
    transient_bytes(pixel_bytes, grid_bytes)
}

fn transient_bytes(pixel_bytes: u64, grid_bytes: u64) -> Result<u64, ColorReconstructionError> {
    pixel_bytes
        .checked_mul(DEVICE_PIXEL_BUFFER_COUNT + HOST_PIXEL_BUFFER_COUNT)
        .and_then(|pixels| {
            grid_bytes
                .checked_mul(GRID_BUFFER_COUNT)
                .and_then(|grids| pixels.checked_add(grids))
        })
        .and_then(|bytes| bytes.checked_add(PARAMS_SIZE))
        .ok_or(ColorReconstructionError::SizeOverflow)
}

impl GpuRuntime {
    /// Runs zero, splat, x/y/z blur, and slice in retained `process_cl` order.
    #[allow(clippy::too_many_lines)]
    pub fn execute_color_reconstruction(
        &self,
        request: ColorReconstructionRequest<'_>,
    ) -> Result<ColorReconstructionResult, ColorReconstructionError> {
        if request.is_cancelled() {
            return Err(ColorReconstructionError::Cancelled);
        }
        if self.is_cpu_only() {
            return Err(ColorReconstructionError::CpuOnly);
        }
        if !matches!(
            self.snapshot().state,
            FaultState::Healthy | FaultState::Degraded
        ) {
            return Err(ColorReconstructionError::Unhealthy);
        }
        let (device, queue) = self.handles().ok_or(ColorReconstructionError::CpuOnly)?;
        if COLORRECONSTRUCTION_SHADER_SOURCE.is_empty() {
            return Err(ColorReconstructionError::ShaderUnavailable);
        }
        let validated = ValidatedRequest::new(request, &device.limits())?;
        if request.is_cancelled() {
            return Err(ColorReconstructionError::Cancelled);
        }

        let input = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RustTable Color Reconstruction input"),
            size: validated.pixel_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        write_mapped_pixels(&input, request.pixels)?;
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RustTable Color Reconstruction output"),
            size: validated.pixel_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RustTable Color Reconstruction readback"),
            size: validated.pixel_bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let grid_usage = wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST;
        let grid_a = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RustTable Color Reconstruction grid"),
            size: validated.grid_bytes,
            usage: grid_usage,
            mapped_at_creation: false,
        });
        let grid_b = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RustTable Color Reconstruction temporary grid"),
            size: validated.grid_bytes,
            usage: grid_usage,
            mapped_at_creation: false,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RustTable Color Reconstruction parameters"),
            size: PARAMS_SIZE,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: true,
        });
        write_mapped_bytes(&params, &pack_params(request, &validated))?;

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RustTable Color Reconstruction bindings"),
            entries: &[
                storage_binding(0, true, validated.pixel_bytes),
                storage_binding(1, false, validated.grid_bytes),
                storage_binding(2, false, validated.grid_bytes),
                storage_binding(3, false, validated.pixel_bytes),
                uniform_binding(4),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("RustTable Color Reconstruction pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("RustTable Color Reconstruction shader"),
            source: wgpu::ShaderSource::Wgsl(COLORRECONSTRUCTION_SHADER_SOURCE.into()),
        });
        let pipelines = STAGE_ORDER.map(|entry_point| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry_point),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some(entry_point),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("RustTable Color Reconstruction group"),
            layout: &layout,
            entries: &[
                buffer_entry(0, &input),
                buffer_entry(1, &grid_a),
                buffer_entry(2, &grid_b),
                buffer_entry(3, &output),
                buffer_entry(4, &params),
            ],
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("RustTable Color Reconstruction encoder"),
        });
        dispatch(
            &mut encoder,
            STAGE_ORDER[0],
            &pipelines[0],
            &group,
            validated.dispatch.zero,
        );
        dispatch(
            &mut encoder,
            STAGE_ORDER[1],
            &pipelines[1],
            &group,
            validated.dispatch.splat,
        );
        encoder.copy_buffer_to_buffer(&grid_a, 0, &grid_b, 0, validated.grid_bytes);
        dispatch(
            &mut encoder,
            STAGE_ORDER[2],
            &pipelines[2],
            &group,
            validated.dispatch.blur_x,
        );
        dispatch(
            &mut encoder,
            STAGE_ORDER[3],
            &pipelines[3],
            &group,
            validated.dispatch.blur_y,
        );
        dispatch(
            &mut encoder,
            STAGE_ORDER[4],
            &pipelines[4],
            &group,
            validated.dispatch.blur_z,
        );
        dispatch(
            &mut encoder,
            STAGE_ORDER[5],
            &pipelines[5],
            &group,
            validated.dispatch.slice,
        );
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, validated.pixel_bytes);
        if request.is_cancelled() {
            return Err(ColorReconstructionError::Cancelled);
        }

        let submission = queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result.map_err(|error| error.to_string()));
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .map_err(|error| ColorReconstructionError::Poll(error.to_string()))?;
        receiver
            .recv()
            .map_err(|error| ColorReconstructionError::Readback(error.to_string()))?
            .map_err(ColorReconstructionError::Readback)?;
        let view = slice
            .get_mapped_range()
            .map_err(|error| ColorReconstructionError::Readback(error.to_string()))?;
        let (chunks, remainder) = view.as_chunks::<16>();
        if !remainder.is_empty() {
            drop(view);
            readback.unmap();
            return Err(ColorReconstructionError::Readback(
                "mapped pixel buffer has a partial LabA value".to_owned(),
            ));
        }
        let pixels = chunks.iter().map(pixel_from_bytes).collect::<Vec<_>>();
        drop(view);
        readback.unmap();
        if request.is_cancelled() {
            return Err(ColorReconstructionError::Cancelled);
        }
        if let Some((pixel, component)) = first_non_finite(&pixels) {
            return Err(ColorReconstructionError::NonFiniteOutput { pixel, component });
        }
        debug_assert_eq!(
            validated.aggregate_transient_bytes,
            transient_bytes(validated.pixel_bytes, validated.grid_bytes)?
        );
        Ok(ColorReconstructionResult {
            pixels,
            dispatches: u32::try_from(STAGE_ORDER.len())
                .map_err(|_| ColorReconstructionError::SizeOverflow)?,
            grid_dimensions: validated.grid_dimensions,
            effective_sigma_s: validated.effective_sigma_s,
            effective_sigma_r: validated.effective_sigma_r,
        })
    }
}

fn validate_parameter(
    name: &'static str,
    value: f32,
    range: std::ops::RangeInclusive<f32>,
) -> Result<(), ColorReconstructionError> {
    if value.is_finite() && range.contains(&value) {
        Ok(())
    } else {
        Err(ColorReconstructionError::InvalidParameter(name))
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn retained_grid_dimension(
    extent: f32,
    sigma: f32,
    maximum: u32,
) -> Result<u32, ColorReconstructionError> {
    let rounded = (extent / sigma).round();
    if !rounded.is_finite() || rounded < i32::MIN as f32 || rounded > i32::MAX as f32 {
        return Err(ColorReconstructionError::SizeOverflow);
    }
    let maximum = i32::try_from(maximum).map_err(|_| ColorReconstructionError::SizeOverflow)?;
    let clamped = (rounded as i32).clamp(4, maximum);
    u32::try_from(clamped)
        .map_err(|_| ColorReconstructionError::SizeOverflow)?
        .checked_add(1)
        .ok_or(ColorReconstructionError::SizeOverflow)
}

fn hue_conversion(hsl_hue: f32) -> f32 {
    let hue = hsl_hue * 6.0;
    let red_hue = if hue < 4.0 { hue + 2.0 } else { hue - 4.0 };
    let blue_hue = if hue > 2.0 { hue - 2.0 } else { hue + 4.0 };
    let red = hue_to_rgb(red_hue);
    let green = hue_to_rgb(hue);
    let blue = hue_to_rgb(blue_hue);

    let xyz = [
        0.436_074_7 * red + 0.385_064_9 * green + 0.143_080_4 * blue,
        0.222_504_5 * red + 0.716_878_6 * green + 0.060_616_9 * blue,
        0.013_932_2 * red + 0.097_104_5 * green + 0.714_173_3 * blue,
    ];
    let d50_inverse = [1.0 / 0.9642, 1.0, 1.0 / 0.8249];
    let epsilon = 216.0 / 24_389.0;
    let kappa = 24_389.0 / 27.0;
    let f: [f32; 3] = std::array::from_fn(|index| {
        let value = xyz[index] * d50_inverse[index];
        if value > epsilon {
            value.cbrt()
        } else {
            (kappa * value + 16.0) / 116.0
        }
    });
    let lab_a = 500.0 * (f[0] - f[1]);
    let lab_b = -200.0 * (f[2] - f[1]);
    lab_b.atan2(lab_a)
}

fn hue_to_rgb(hue: f32) -> f32 {
    if hue < 1.0 {
        hue
    } else if hue < 3.0 {
        1.0
    } else if hue < 4.0 {
        4.0 - hue
    } else {
        0.0
    }
}

fn pack_params(
    request: ColorReconstructionRequest<'_>,
    validated: &ValidatedRequest,
) -> [u8; PARAMS_BYTES] {
    let [size_x, size_y, size_z] = validated.grid_dimensions;
    let words = [
        validated.width,
        validated.height,
        size_x,
        size_y,
        size_z,
        size_x * 4,
        size_y * size_z,
        request.precedence as u32,
        validated.effective_sigma_s.to_bits(),
        validated.effective_sigma_r.to_bits(),
        request.threshold.to_bits(),
        validated.lch_hue.to_bits(),
        (std::f32::consts::PI * std::f32::consts::PI / 8.0).to_bits(),
        u32::from_le_bytes(request.roi.x.to_le_bytes()),
        u32::from_le_bytes(request.roi.y.to_le_bytes()),
        u32::from_le_bytes(request.roi.x.to_le_bytes()),
        u32::from_le_bytes(request.roi.y.to_le_bytes()),
        validated.rescale.to_bits(),
        validated.pixel_count,
        validated.grid_cells,
    ];
    let mut bytes = [0_u8; PARAMS_BYTES];
    let (chunks, remainder) = bytes.as_chunks_mut::<4>();
    debug_assert!(remainder.is_empty());
    for (chunk, word) in chunks.iter_mut().zip(words) {
        chunk.copy_from_slice(&word.to_le_bytes());
    }
    bytes
}

fn storage_binding(binding: u32, read_only: bool, size: u64) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: NonZeroU64::new(size),
        },
        count: None,
    }
}

fn uniform_binding(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: NonZeroU64::new(PARAMS_SIZE),
        },
        count: None,
    }
}

fn buffer_entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

fn dispatch(
    encoder: &mut wgpu::CommandEncoder,
    label: &str,
    pipeline: &wgpu::ComputePipeline,
    group: &wgpu::BindGroup,
    workgroups: [u32; 3],
) {
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some(label),
        timestamp_writes: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, group, &[]);
    pass.dispatch_workgroups(workgroups[0], workgroups[1], workgroups[2]);
}

fn write_mapped_pixels(
    buffer: &wgpu::Buffer,
    pixels: &[[f32; 4]],
) -> Result<(), ColorReconstructionError> {
    let slice = buffer.slice(..);
    let mut view = slice
        .get_mapped_range_mut()
        .map_err(|error| ColorReconstructionError::Upload(error.to_string()))?;
    let expected = pixels
        .len()
        .checked_mul(PIXEL_BYTES_USIZE)
        .ok_or(ColorReconstructionError::SizeOverflow)?;
    if view.len() != expected {
        drop(view);
        buffer.unmap();
        return Err(ColorReconstructionError::Upload(
            "mapped upload buffer does not match LabA pixel packing".to_owned(),
        ));
    }
    for (pixel_index, pixel) in pixels.iter().enumerate() {
        let start = pixel_index
            .checked_mul(PIXEL_BYTES_USIZE)
            .ok_or(ColorReconstructionError::SizeOverflow)?;
        for (component_index, component) in pixel.iter().enumerate() {
            let component_start = start
                .checked_add(component_index * 4)
                .ok_or(ColorReconstructionError::SizeOverflow)?;
            view.slice(component_start..component_start + 4)
                .copy_from_slice(&component.to_le_bytes());
        }
    }
    drop(view);
    buffer.unmap();
    Ok(())
}

fn write_mapped_bytes(buffer: &wgpu::Buffer, bytes: &[u8]) -> Result<(), ColorReconstructionError> {
    let slice = buffer.slice(..);
    let mut view = slice
        .get_mapped_range_mut()
        .map_err(|error| ColorReconstructionError::Upload(error.to_string()))?;
    if view.len() != bytes.len() {
        drop(view);
        buffer.unmap();
        return Err(ColorReconstructionError::Upload(
            "mapped upload buffer does not match parameter length".to_owned(),
        ));
    }
    view.copy_from_slice(bytes);
    drop(view);
    buffer.unmap();
    Ok(())
}

fn first_non_finite(pixels: &[[f32; 4]]) -> Option<(usize, usize)> {
    pixels.iter().enumerate().find_map(|(pixel, components)| {
        components
            .iter()
            .position(|component| !component.is_finite())
            .map(|component| (pixel, component))
    })
}

fn pixel_from_bytes(bytes: &[u8; 16]) -> [f32; 4] {
    std::array::from_fn(|component| {
        let start = component * 4;
        f32::from_le_bytes(
            bytes[start..start + 4]
                .try_into()
                .expect("LabA component has four bytes"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_BUDGET: u64 = 512 * 1024 * 1024;

    fn request(pixels: &[[f32; 4]], width: usize, height: usize) -> ColorReconstructionRequest<'_> {
        ColorReconstructionRequest::new(
            pixels,
            ColorReconstructionRoi::new(0, 0, width, height, 1.0),
            1.0,
            100.0,
            400.0,
            10.0,
            0.66,
            ColorReconstructionPrecedence::None,
            TEST_BUDGET,
        )
    }

    #[test]
    fn shader_retains_six_stages_and_fixed_workgroups() {
        let module = naga::front::wgsl::parse_str(COLORRECONSTRUCTION_SHADER_SOURCE)
            .expect("Color Reconstruction WGSL syntax");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("Color Reconstruction WGSL validation");
        let entries = module
            .entry_points
            .iter()
            .map(|entry| (entry.name.as_str(), entry.workgroup_size))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(entries.len(), STAGE_ORDER.len());
        for entry in STAGE_ORDER {
            assert_eq!(entries[entry], [16, 16, 1]);
        }
    }

    #[test]
    fn splat_reduction_preserves_native_overwrite_runs() {
        assert!(
            COLORRECONSTRUCTION_SHADER_SOURCE
                .contains("accumulated = local_accumulator[line_index];")
        );
        assert!(
            !COLORRECONSTRUCTION_SHADER_SOURCE
                .contains("accumulated += local_accumulator[line_index];")
        );
    }

    #[test]
    fn retained_grid_geometry_clamps_then_adds_guard_cells() {
        let pixels = vec![[50.0, 0.0, 0.0, 1.0]; 400 * 300];
        let validated = ValidatedRequest::new(
            request(&pixels, 400, 300),
            &wgpu::Limits::downlevel_defaults(),
        )
        .expect("geometry");
        assert_eq!(validated.grid_dimensions, [5, 5, 11]);
        assert_eq!(validated.effective_sigma_s.to_bits(), 100.0_f32.to_bits());
        assert_eq!(validated.effective_sigma_r.to_bits(), 10.0_f32.to_bits());

        assert_eq!(
            retained_grid_dimension(10_000.0, 1.0, COLORRECONSTRUCTION_MAX_SPATIAL_RESOLUTION)
                .unwrap(),
            501
        );
        assert_eq!(
            retained_grid_dimension(100.0, 0.1, COLORRECONSTRUCTION_MAX_RANGE_RESOLUTION).unwrap(),
            101
        );
    }

    #[test]
    fn dispatch_geometry_and_order_match_process_cl() {
        let pixels = vec![[50.0, 0.0, 0.0, 1.0]; 400 * 300];
        let validated = ValidatedRequest::new(
            request(&pixels, 400, 300),
            &wgpu::Limits::downlevel_defaults(),
        )
        .expect("dispatches");
        assert_eq!(STAGE_ORDER[0], "colorreconstruction_zero");
        assert_eq!(STAGE_ORDER[5], "colorreconstruction_slice");
        assert_eq!(
            validated.dispatch.ordered(),
            [
                [2, 4, 1],
                [25, 19, 1],
                [1, 1, 1],
                [1, 1, 1],
                [1, 1, 1],
                [25, 19, 1],
            ]
        );
    }

    #[test]
    fn exact_memory_budget_counts_owned_buffers() {
        let roi = ColorReconstructionRoi::new(0, 0, 1, 1, 1.0);
        let required =
            colorreconstruction_transient_memory_bytes(roi, 1.0, 400.0, 10.0).expect("footprint");
        let grid_bytes = 5_u64 * 5 * 11 * GRID_CELL_BYTES;
        assert_eq!(required, 4 * PIXEL_BYTES + 2 * grid_bytes + PARAMS_SIZE);
        let pixels = [[50.0, 0.0, 0.0, 1.0]];
        let under_budget = ColorReconstructionRequest::new(
            &pixels,
            roi,
            1.0,
            100.0,
            400.0,
            10.0,
            0.66,
            ColorReconstructionPrecedence::None,
            required - 1,
        );
        assert!(matches!(
            ValidatedRequest::new(under_budget, &wgpu::Limits::downlevel_defaults()),
            Err(ColorReconstructionError::AggregateMemoryLimit {
                required: actual,
                limit
            }) if actual == required && limit == required - 1
        ));
    }

    #[test]
    fn qualification_rejects_shape_parameter_and_finite_violations() {
        let pixels = [[50.0, 0.0, 0.0, 1.0]];
        assert!(matches!(
            ValidatedRequest::new(request(&pixels, 2, 1), &wgpu::Limits::downlevel_defaults()),
            Err(ColorReconstructionError::BufferShape { .. })
        ));
        let invalid_threshold = ColorReconstructionRequest::new(
            &pixels,
            ColorReconstructionRoi::new(0, 0, 1, 1, 1.0),
            1.0,
            49.0,
            400.0,
            10.0,
            0.66,
            ColorReconstructionPrecedence::None,
            TEST_BUDGET,
        );
        assert!(matches!(
            ValidatedRequest::new(invalid_threshold, &wgpu::Limits::downlevel_defaults()),
            Err(ColorReconstructionError::InvalidParameter("threshold"))
        ));
        let invalid = [[50.0, f32::NAN, 0.0, 1.0]];
        assert!(matches!(
            ValidatedRequest::new(request(&invalid, 1, 1), &wgpu::Limits::downlevel_defaults()),
            Err(ColorReconstructionError::NonFiniteInput {
                pixel: 0,
                component: 1
            })
        ));
    }

    #[test]
    fn slice_rescale_retains_process_cl_operation_order() {
        let pixels = [[50.0, 0.0, 0.0, 1.0]];
        let request = ColorReconstructionRequest::new(
            &pixels,
            ColorReconstructionRoi::new(0, 0, 1, 1, 2.0),
            3.0,
            100.0,
            400.0,
            10.0,
            0.66,
            ColorReconstructionPrecedence::None,
            TEST_BUDGET,
        );
        let validated =
            ValidatedRequest::new(request, &wgpu::Limits::downlevel_defaults()).expect("rescale");
        let expected = 3.0_f32 / (2.0 * (3.0 / 2.0));
        assert_eq!(validated.rescale.to_bits(), expected.to_bits());
        let packed = pack_params(request, &validated);
        assert_eq!(
            f32::from_le_bytes(packed[68..72].try_into().expect("rescale word")).to_bits(),
            expected.to_bits()
        );
    }

    #[test]
    fn hue_conversion_retains_hsl_to_rec709_d50_to_lab_path() {
        let red = hue_conversion(0.0);
        let green = hue_conversion(1.0 / 3.0);
        let blue = hue_conversion(2.0 / 3.0);
        assert!(red > 0.0);
        assert!(green > red);
        assert!(blue < 0.0);
        assert_ne!(red.to_bits(), green.to_bits());
    }

    #[test]
    fn cancellation_does_not_mutate_qualified_geometry() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let pixels = [[50.0, 0.0, 0.0, 1.0]];
        let request = request(&pixels, 1, 1).with_cancellation(&cancellation);
        assert!(request.is_cancelled());
        ValidatedRequest::new(request, &wgpu::Limits::downlevel_defaults())
            .expect("cancelled request geometry remains valid");
    }

    #[tokio::test]
    async fn fully_clipped_input_preserves_laba_when_backend_is_available() {
        let Ok(runtime) = GpuRuntime::initialize(crate::GpuRuntimeConfig::default()).await else {
            return;
        };
        if runtime.is_cpu_only() {
            return;
        }
        let input = [
            [100.0, 10.0, -20.0, 0.25],
            [120.0, -5.0, 8.0, 0.75],
            [150.0, 1.0, 2.0, 1.0],
            [100.0, -3.0, -4.0, 0.5],
        ];
        let result = runtime
            .execute_color_reconstruction(ColorReconstructionRequest::new(
                &input,
                ColorReconstructionRoi::new(-7, 11, 2, 2, 1.0),
                1.0,
                100.0,
                400.0,
                10.0,
                0.66,
                ColorReconstructionPrecedence::Hue,
                TEST_BUDGET,
            ))
            .expect("Color Reconstruction dispatch");
        assert_eq!(result.dispatches(), 6);
        assert_eq!(result.grid_dimensions(), [5, 5, 11]);
        assert_eq!(result.pixels(), input);
    }
}
