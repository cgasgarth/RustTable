//! Safe WGPU port of Darktable's `src/common/bilateralcl.c`,
//! `src/common/bilateralcl.h`, and `data/kernels/bilateral.cl`.

use std::fmt;
use std::num::NonZeroU64;

use crate::shader::{
    BILATERAL_ENTRY_CONTRACTS, BilateralEntryContract, BindingContract, ShaderEntry, ShaderRegistry,
};
use crate::{CancellationToken, FaultState, GpuRuntime};

const PARAMS_SIZE: u64 = 64;
const PIXEL_BYTES: usize = 16;
const SPLAT_WORKGROUP_STORAGE: u32 = 9_216;
const BILATERAL_ENTRY_COUNT: usize = 6;
const REPLACE_PIXEL_BUFFER_COUNT: u64 = 3;
const ADDITIVE_PIXEL_BUFFER_COUNT: u64 = 4;
const GRID_BUFFER_COUNT: u64 = 2;
const HOST_TRANSIENT_PIXEL_BUFFER_COUNT: u64 = 1;

#[derive(Debug, Clone, Copy)]
struct RuntimeEntry {
    shader: &'static ShaderEntry,
    contract: &'static BilateralEntryContract,
}

#[derive(Debug, Clone, Copy)]
struct BilateralProgram {
    entries: [RuntimeEntry; BILATERAL_ENTRY_COUNT],
    source: &'static str,
}

impl BilateralProgram {
    fn checked() -> Result<Self, BilateralGridError> {
        let registry = ShaderRegistry::checked_in();
        let entries: [RuntimeEntry; BILATERAL_ENTRY_COUNT] = BILATERAL_ENTRY_CONTRACTS
            .iter()
            .map(|contract| {
                let shader = registry
                    .find("rusttable.bilateral", contract.entry_point)
                    .ok_or(BilateralGridError::ShaderUnavailable)?;
                Ok(RuntimeEntry { shader, contract })
            })
            .collect::<Result<Vec<_>, BilateralGridError>>()?
            .try_into()
            .map_err(|_| BilateralGridError::ShaderUnavailable)?;
        let source = entries[0].shader.expanded_source.as_str();
        if source.is_empty()
            || entries.iter().any(|entry| {
                entry.shader.expanded_source != source
                    || entry.shader.reflection.entry_point != entry.contract.entry_point
                    || entry.shader.reflection.workgroup_size != entry.contract.workgroup_size
            })
        {
            return Err(BilateralGridError::ShaderUnavailable);
        }
        Ok(Self { entries, source })
    }

    fn entry(self, entry_point: &str) -> Result<RuntimeEntry, BilateralGridError> {
        self.entries
            .into_iter()
            .find(|entry| entry.contract.entry_point == entry_point)
            .ok_or(BilateralGridError::ShaderUnavailable)
    }

    fn maximum_binding_count(self) -> u32 {
        self.entries
            .iter()
            .flat_map(|entry| entry.contract.bindings)
            .map(|binding| binding.binding.saturating_add(1))
            .max()
            .unwrap_or(0)
    }

    fn maximum_storage_binding_count(self) -> u32 {
        self.entries
            .iter()
            .map(|entry| {
                u32::try_from(
                    entry
                        .contract
                        .bindings
                        .iter()
                        .filter(|binding| binding.storage)
                        .count(),
                )
                .unwrap_or(u32::MAX)
            })
            .max()
            .unwrap_or(0)
    }

    fn maximum_uniform_binding_count(self) -> u32 {
        self.entries
            .iter()
            .map(|entry| {
                u32::try_from(
                    entry
                        .contract
                        .bindings
                        .iter()
                        .filter(|binding| !binding.storage)
                        .count(),
                )
                .unwrap_or(u32::MAX)
            })
            .max()
            .unwrap_or(0)
    }

    fn maximum_uniform_binding_size(self) -> u32 {
        self.entries
            .iter()
            .flat_map(|entry| entry.contract.bindings)
            .filter(|binding| !binding.storage)
            .map(|binding| binding.minimum_binding_size)
            .max()
            .unwrap_or(0)
    }

    fn maximum_workgroup_dimension(self, axis: usize) -> u32 {
        self.entries
            .iter()
            .map(|entry| entry.contract.workgroup_size[axis])
            .max()
            .unwrap_or(0)
    }

    fn maximum_workgroup_invocations(self) -> u32 {
        self.entries
            .iter()
            .map(|entry| {
                entry
                    .contract
                    .workgroup_size
                    .into_iter()
                    .try_fold(1_u32, u32::checked_mul)
                    .unwrap_or(u32::MAX)
            })
            .max()
            .unwrap_or(0)
    }
}

/// Selects the retained bilateral slicing behavior.
#[derive(Debug, Clone, Copy)]
pub enum BilateralSlice<'a> {
    /// Copy the guide pixel and replace its lightness with the sliced result.
    Replace,
    /// Add the guide-derived correction to this target's lightness.
    AddTo(&'a [[f32; 4]]),
}

/// A full-frame bilateral-grid request with geometry already resolved by the
/// canonical CPU geometry helper.
///
/// This boundary deliberately accepts primitive geometry instead of
/// recomputing Darktable's clamping formulas in the GPU crate.
#[derive(Debug, Clone, Copy)]
pub struct BilateralGridRequest<'a> {
    pixels: &'a [[f32; 4]],
    width: usize,
    height: usize,
    grid_dimensions: [usize; 3],
    effective_sigma_s: f32,
    effective_sigma_r: f32,
    detail: f32,
    slice: BilateralSlice<'a>,
    transient_memory_budget_bytes: u64,
    cancellation: Option<&'a CancellationToken>,
}

impl<'a> BilateralGridRequest<'a> {
    /// Builds a request matching retained `dt_bilateral_slice_cl`.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn slice(
        pixels: &'a [[f32; 4]],
        width: usize,
        height: usize,
        grid_dimensions: [usize; 3],
        effective_sigma_s: f32,
        effective_sigma_r: f32,
        detail: f32,
        transient_memory_budget_bytes: u64,
    ) -> Self {
        Self {
            pixels,
            width,
            height,
            grid_dimensions,
            effective_sigma_s,
            effective_sigma_r,
            detail,
            slice: BilateralSlice::Replace,
            transient_memory_budget_bytes,
            cancellation: None,
        }
    }

    /// Builds a request matching retained `dt_bilateral_slice_to_output_cl`.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn slice_to_output(
        pixels: &'a [[f32; 4]],
        target: &'a [[f32; 4]],
        width: usize,
        height: usize,
        grid_dimensions: [usize; 3],
        effective_sigma_s: f32,
        effective_sigma_r: f32,
        detail: f32,
        transient_memory_budget_bytes: u64,
    ) -> Self {
        Self {
            pixels,
            width,
            height,
            grid_dimensions,
            effective_sigma_s,
            effective_sigma_r,
            detail,
            slice: BilateralSlice::AddTo(target),
            transient_memory_budget_bytes,
            cancellation: None,
        }
    }

    /// Attaches a signal checked before validation, before submission, and
    /// after readback.
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

/// Read-back pixels and the number of compute dispatches submitted.
#[derive(Debug, Clone, PartialEq)]
pub struct BilateralGridResult {
    pixels: Vec<[f32; 4]>,
    dispatches: u32,
}

impl BilateralGridResult {
    #[must_use]
    pub fn pixels(&self) -> &[[f32; 4]] {
        &self.pixels
    }

    #[must_use]
    pub const fn dispatches(&self) -> u32 {
        self.dispatches
    }

    #[must_use]
    pub fn into_pixels(self) -> Vec<[f32; 4]> {
        self.pixels
    }
}

/// Checked failure from the concrete bilateral WGPU path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BilateralGridError {
    CpuOnly,
    Unhealthy,
    BufferShape { expected: usize, actual: usize },
    TargetShape { expected: usize, actual: usize },
    InvalidGeometry,
    NonFiniteInput { pixel: usize, component: usize },
    NonFiniteTarget { pixel: usize, component: usize },
    NonFiniteOutput { pixel: usize, component: usize },
    NonFiniteDetail,
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

impl fmt::Display for BilateralGridError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CpuOnly => formatter.write_str("bilateral WGPU execution is CPU-only"),
            Self::Unhealthy => formatter.write_str("bilateral WGPU execution is unhealthy"),
            Self::BufferShape { expected, actual } => {
                write!(
                    formatter,
                    "bilateral WGPU expected {expected} guide pixels, got {actual}"
                )
            }
            Self::TargetShape { expected, actual } => {
                write!(
                    formatter,
                    "bilateral WGPU expected {expected} target pixels, got {actual}"
                )
            }
            Self::InvalidGeometry => {
                formatter.write_str("bilateral WGPU geometry or effective sigma is invalid")
            }
            Self::NonFiniteInput { pixel, component } => {
                write!(
                    formatter,
                    "bilateral WGPU guide has a non-finite component {component} at pixel {pixel}"
                )
            }
            Self::NonFiniteTarget { pixel, component } => {
                write!(
                    formatter,
                    "bilateral WGPU target has a non-finite component {component} at pixel {pixel}"
                )
            }
            Self::NonFiniteOutput { pixel, component } => {
                write!(
                    formatter,
                    "bilateral WGPU produced a non-finite component {component} at pixel {pixel}"
                )
            }
            Self::NonFiniteDetail => formatter.write_str("bilateral WGPU detail is non-finite"),
            Self::Cancelled => formatter.write_str("bilateral WGPU execution was cancelled"),
            Self::SizeOverflow => formatter.write_str("bilateral WGPU size overflowed"),
            Self::InvalidMemoryBudget => {
                formatter.write_str("bilateral WGPU transient-memory budget must be nonzero")
            }
            Self::BufferLimit { required, limit } => {
                write!(
                    formatter,
                    "bilateral WGPU buffer requires {required} bytes; device limit is {limit}"
                )
            }
            Self::AggregateMemoryLimit { required, limit } => {
                write!(
                    formatter,
                    "bilateral WGPU transient allocations require {required} bytes; transient-memory budget is {limit}"
                )
            }
            Self::InsufficientComputeLimits => formatter
                .write_str("bilateral WGPU device compute or binding limits are insufficient"),
            Self::TooManyWorkgroups => {
                formatter.write_str("bilateral WGPU dispatch exceeds the device workgroup limit")
            }
            Self::ShaderUnavailable => formatter.write_str("bilateral WGPU shader is unavailable"),
            Self::Upload(error) => write!(formatter, "bilateral WGPU upload failed: {error}"),
            Self::Poll(error) => write!(formatter, "bilateral WGPU poll failed: {error}"),
            Self::Readback(error) => {
                write!(formatter, "bilateral WGPU readback failed: {error}")
            }
        }
    }
}

impl std::error::Error for BilateralGridError {}

#[derive(Debug, Clone, Copy)]
struct DispatchPlan {
    workgroups: [u32; 3],
    strides: [u32; 3],
    sizes: [u32; 3],
}

#[derive(Debug)]
struct ValidatedRequest {
    width: u32,
    height: u32,
    grid_dimensions: [u32; 3],
    pixel_count: u32,
    grid_values: u32,
    pixel_bytes: u64,
    grid_bytes: u64,
    effective_sigma_s: f32,
    effective_sigma_r: f32,
    detail: f32,
    dispatches: [DispatchPlan; 6],
}

impl ValidatedRequest {
    #[cfg(test)]
    fn new(
        request: BilateralGridRequest<'_>,
        limits: &wgpu::Limits,
    ) -> Result<Self, BilateralGridError> {
        let program = BilateralProgram::checked()?;
        Self::new_with_program(request, limits, program)
    }

    fn new_with_program(
        request: BilateralGridRequest<'_>,
        limits: &wgpu::Limits,
        program: BilateralProgram,
    ) -> Result<Self, BilateralGridError> {
        let expected = request
            .width
            .checked_mul(request.height)
            .ok_or(BilateralGridError::SizeOverflow)?;
        if request.width == 0 || request.height == 0 {
            return Err(BilateralGridError::InvalidGeometry);
        }
        if request.pixels.len() != expected {
            return Err(BilateralGridError::BufferShape {
                expected,
                actual: request.pixels.len(),
            });
        }
        if let Some((pixel, component)) = first_non_finite(request.pixels) {
            return Err(BilateralGridError::NonFiniteInput { pixel, component });
        }
        if !request.detail.is_finite() {
            return Err(BilateralGridError::NonFiniteDetail);
        }
        if !request.effective_sigma_s.is_finite()
            || request.effective_sigma_s <= 0.0
            || !request.effective_sigma_r.is_finite()
            || request.effective_sigma_r <= 0.0
            || request
                .grid_dimensions
                .iter()
                .any(|dimension| *dimension < 2)
        {
            return Err(BilateralGridError::InvalidGeometry);
        }
        let sigma_s_squared = request.effective_sigma_s * request.effective_sigma_s;
        if !sigma_s_squared.is_finite()
            || sigma_s_squared <= 0.0
            || !(100.0 / sigma_s_squared).is_finite()
            || !request.effective_sigma_s.recip().is_finite()
            || !request.effective_sigma_r.recip().is_finite()
        {
            return Err(BilateralGridError::InvalidGeometry);
        }
        if !(-request.detail * request.effective_sigma_r * 0.04).is_finite() {
            return Err(BilateralGridError::NonFiniteDetail);
        }
        if let BilateralSlice::AddTo(target) = request.slice {
            if target.len() != expected {
                return Err(BilateralGridError::TargetShape {
                    expected,
                    actual: target.len(),
                });
            }
            if let Some((pixel, component)) = first_non_finite(target) {
                return Err(BilateralGridError::NonFiniteTarget { pixel, component });
            }
        }
        if request.transient_memory_budget_bytes == 0 {
            return Err(BilateralGridError::InvalidMemoryBudget);
        }

        let width = u32::try_from(request.width).map_err(|_| BilateralGridError::SizeOverflow)?;
        let height = u32::try_from(request.height).map_err(|_| BilateralGridError::SizeOverflow)?;
        let pixel_count = u32::try_from(expected).map_err(|_| BilateralGridError::SizeOverflow)?;
        let grid_dimensions = request.grid_dimensions.map(u32::try_from);
        let [size_x, size_y, size_z] = [
            grid_dimensions[0].map_err(|_| BilateralGridError::SizeOverflow)?,
            grid_dimensions[1].map_err(|_| BilateralGridError::SizeOverflow)?,
            grid_dimensions[2].map_err(|_| BilateralGridError::SizeOverflow)?,
        ];
        let xy = size_x
            .checked_mul(size_y)
            .ok_or(BilateralGridError::SizeOverflow)?;
        let grid_values = xy
            .checked_mul(size_z)
            .ok_or(BilateralGridError::SizeOverflow)?;
        let zero_height = size_y
            .checked_mul(size_z)
            .ok_or(BilateralGridError::SizeOverflow)?;
        let pixel_bytes = u64::try_from(
            expected
                .checked_mul(PIXEL_BYTES)
                .ok_or(BilateralGridError::SizeOverflow)?,
        )
        .map_err(|_| BilateralGridError::SizeOverflow)?;
        let grid_bytes = u64::from(grid_values)
            .checked_mul(4)
            .ok_or(BilateralGridError::SizeOverflow)?;

        let binding_limit = limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size);
        for required in [pixel_bytes, grid_bytes] {
            if required > binding_limit {
                return Err(BilateralGridError::BufferLimit {
                    required,
                    limit: binding_limit,
                });
            }
        }
        let uniform_buffer_limit = limits
            .max_uniform_buffer_binding_size
            .min(limits.max_buffer_size);
        if PARAMS_SIZE > uniform_buffer_limit {
            return Err(BilateralGridError::BufferLimit {
                required: PARAMS_SIZE,
                limit: uniform_buffer_limit,
            });
        }
        let aggregate_transient_bytes =
            aggregate_transient_bytes(pixel_bytes, grid_bytes, request.slice)?;
        if aggregate_transient_bytes > request.transient_memory_budget_bytes {
            return Err(BilateralGridError::AggregateMemoryLimit {
                required: aggregate_transient_bytes,
                limit: request.transient_memory_budget_bytes,
            });
        }
        if limits.max_compute_invocations_per_workgroup < program.maximum_workgroup_invocations()
            || limits.max_compute_workgroup_size_x < program.maximum_workgroup_dimension(0)
            || limits.max_compute_workgroup_size_y < program.maximum_workgroup_dimension(1)
            || limits.max_compute_workgroup_size_z < program.maximum_workgroup_dimension(2)
            || limits.max_compute_workgroup_storage_size < SPLAT_WORKGROUP_STORAGE
            || limits.max_bind_groups < 1
            || limits.max_bindings_per_bind_group < program.maximum_binding_count()
            || limits.max_storage_buffers_per_shader_stage < program.maximum_storage_binding_count()
            || limits.max_uniform_buffers_per_shader_stage < program.maximum_uniform_binding_count()
            || limits.max_uniform_buffer_binding_size
                < u64::from(program.maximum_uniform_binding_size())
        {
            return Err(BilateralGridError::InsufficientComputeLimits);
        }

        let zero_workgroup = program.entry("zero")?.contract.workgroup_size;
        let splat_workgroup = program.entry("splat")?.contract.workgroup_size;
        let blur_workgroup = program.entry("blur_line")?.contract.workgroup_size;
        let blur_z_workgroup = program.entry("blur_line_z")?.contract.workgroup_size;
        let slice_workgroup = match request.slice {
            BilateralSlice::Replace => program.entry("slice")?.contract.workgroup_size,
            BilateralSlice::AddTo(_) => program.entry("slice_to_output")?.contract.workgroup_size,
        };
        let dispatches = [
            DispatchPlan {
                workgroups: [
                    size_x.div_ceil(zero_workgroup[0]),
                    zero_height.div_ceil(zero_workgroup[1]),
                    1,
                ],
                strides: [1, size_x, 0],
                sizes: [size_x, zero_height, 1],
            },
            DispatchPlan {
                workgroups: [
                    width.div_ceil(splat_workgroup[0]),
                    height.div_ceil(splat_workgroup[1]),
                    1,
                ],
                strides: [1, size_x, xy],
                sizes: [width, height, 1],
            },
            DispatchPlan {
                workgroups: [
                    size_x.div_ceil(blur_workgroup[0]),
                    size_y.div_ceil(blur_workgroup[1]),
                    size_z,
                ],
                strides: [xy, size_x, 1],
                sizes: [size_z, size_y, size_x],
            },
            DispatchPlan {
                workgroups: [
                    size_y.div_ceil(blur_workgroup[0]),
                    size_x.div_ceil(blur_workgroup[1]),
                    size_z,
                ],
                strides: [xy, 1, size_x],
                sizes: [size_z, size_x, size_y],
            },
            DispatchPlan {
                workgroups: [
                    size_z.div_ceil(blur_z_workgroup[0]),
                    size_y.div_ceil(blur_z_workgroup[1]),
                    size_x,
                ],
                strides: [1, size_x, xy],
                sizes: [size_x, size_y, size_z],
            },
            DispatchPlan {
                workgroups: [
                    width.div_ceil(slice_workgroup[0]),
                    height.div_ceil(slice_workgroup[1]),
                    1,
                ],
                strides: [1, size_x, xy],
                sizes: [width, height, 1],
            },
        ];
        let workgroup_limit = limits.max_compute_workgroups_per_dimension;
        if dispatches
            .iter()
            .flat_map(|dispatch| dispatch.workgroups)
            .any(|workgroups| workgroups > workgroup_limit)
        {
            return Err(BilateralGridError::TooManyWorkgroups);
        }

        Ok(Self {
            width,
            height,
            grid_dimensions: [size_x, size_y, size_z],
            pixel_count,
            grid_values,
            pixel_bytes,
            grid_bytes,
            effective_sigma_s: request.effective_sigma_s,
            effective_sigma_r: request.effective_sigma_r,
            detail: request.detail,
            dispatches,
        })
    }
}

fn aggregate_transient_bytes(
    pixel_bytes: u64,
    grid_bytes: u64,
    slice: BilateralSlice<'_>,
) -> Result<u64, BilateralGridError> {
    let pixel_buffer_count = match slice {
        BilateralSlice::Replace => REPLACE_PIXEL_BUFFER_COUNT,
        BilateralSlice::AddTo(_) => ADDITIVE_PIXEL_BUFFER_COUNT,
    };
    let parameter_buffer_count =
        u64::try_from(BILATERAL_ENTRY_COUNT).map_err(|_| BilateralGridError::SizeOverflow)?;

    // This accounts for every allocation owned explicitly by this boundary:
    // guide/output/readback device buffers, an optional additive target, two
    // grids, one uniform buffer per dispatch, and the returned pixel-sized host
    // vector. Pixel and parameter uploads write directly into buffers mapped at
    // creation, avoiding both serialized upload vectors and queue-owned staging.
    pixel_bytes
        .checked_mul(pixel_buffer_count)
        .and_then(|pixels| {
            grid_bytes
                .checked_mul(GRID_BUFFER_COUNT)
                .and_then(|grids| pixels.checked_add(grids))
        })
        .and_then(|resident| {
            PARAMS_SIZE
                .checked_mul(parameter_buffer_count)
                .and_then(|parameters| resident.checked_add(parameters))
        })
        .and_then(|resident| {
            pixel_bytes
                .checked_mul(HOST_TRANSIENT_PIXEL_BUFFER_COUNT)
                .and_then(|host| resident.checked_add(host))
        })
        .ok_or(BilateralGridError::SizeOverflow)
}

impl GpuRuntime {
    /// Runs zero, splat, x blur, y blur, z derivative, and slice for one
    /// checked full-frame request.
    #[allow(clippy::too_many_lines)]
    pub fn execute_bilateral_grid(
        &self,
        request: BilateralGridRequest<'_>,
    ) -> Result<BilateralGridResult, BilateralGridError> {
        if request.is_cancelled() {
            return Err(BilateralGridError::Cancelled);
        }
        if self.is_cpu_only() {
            return Err(BilateralGridError::CpuOnly);
        }
        if !matches!(
            self.snapshot().state,
            FaultState::Healthy | FaultState::Degraded
        ) {
            return Err(BilateralGridError::Unhealthy);
        }
        let (device, queue) = self.handles().ok_or(BilateralGridError::CpuOnly)?;
        let program = BilateralProgram::checked()?;
        let validated = ValidatedRequest::new_with_program(request, &device.limits(), program)?;
        let zero_entry = program.entry("zero")?;
        let splat_entry = program.entry("splat")?;
        let blur_entry = program.entry("blur_line")?;
        let blur_z_entry = program.entry("blur_line_z")?;
        let slice_entry = match request.slice {
            BilateralSlice::Replace => program.entry("slice")?,
            BilateralSlice::AddTo(_) => program.entry("slice_to_output")?,
        };

        let storage_usage = wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST;
        let input = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RustTable bilateral guide"),
            size: validated.pixel_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        write_mapped_pixels(&input, request.pixels)?;
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RustTable bilateral output"),
            size: validated.pixel_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let grid_a = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RustTable bilateral grid A"),
            size: validated.grid_bytes,
            usage: storage_usage,
            mapped_at_creation: false,
        });
        let grid_b = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RustTable bilateral grid B"),
            size: validated.grid_bytes,
            usage: storage_usage,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RustTable bilateral readback"),
            size: validated.pixel_bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let target = if let BilateralSlice::AddTo(pixels) = request.slice {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("RustTable bilateral target"),
                size: validated.pixel_bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: true,
            });
            write_mapped_pixels(&buffer, pixels)?;
            Some(buffer)
        } else {
            None
        };

        let parameter_buffers = validated
            .dispatches
            .iter()
            .map(|dispatch| {
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("RustTable bilateral parameters"),
                    size: PARAMS_SIZE,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: true,
                });
                write_mapped_bytes(&buffer, &pack_params(&validated, *dispatch))?;
                Ok(buffer)
            })
            .collect::<Result<Vec<_>, BilateralGridError>>()?;

        let zero_layout = bilateral_layout(
            device,
            "RustTable bilateral zero bindings",
            zero_entry.contract,
            &validated,
        )?;
        let splat_layout = bilateral_layout(
            device,
            "RustTable bilateral splat bindings",
            splat_entry.contract,
            &validated,
        )?;
        let blur_layout = bilateral_layout(
            device,
            "RustTable bilateral blur bindings",
            blur_entry.contract,
            &validated,
        )?;
        let slice_layout = bilateral_layout(
            device,
            "RustTable bilateral slice bindings",
            slice_entry.contract,
            &validated,
        )?;
        let zero_pipeline_layout = pipeline_layout(device, "bilateral zero layout", &zero_layout);
        let splat_pipeline_layout =
            pipeline_layout(device, "bilateral splat layout", &splat_layout);
        let blur_pipeline_layout = pipeline_layout(device, "bilateral blur layout", &blur_layout);
        let slice_pipeline_layout =
            pipeline_layout(device, "bilateral slice layout", &slice_layout);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("RustTable bilateral shader"),
            source: wgpu::ShaderSource::Wgsl(program.source.into()),
        });
        let zero_pipeline = compute_pipeline(
            device,
            &zero_pipeline_layout,
            &shader,
            zero_entry.contract.entry_point,
        );
        let splat_pipeline = compute_pipeline(
            device,
            &splat_pipeline_layout,
            &shader,
            splat_entry.contract.entry_point,
        );
        let blur_pipeline = compute_pipeline(
            device,
            &blur_pipeline_layout,
            &shader,
            blur_entry.contract.entry_point,
        );
        let blur_z_pipeline = compute_pipeline(
            device,
            &blur_pipeline_layout,
            &shader,
            blur_z_entry.contract.entry_point,
        );
        let slice_pipeline = compute_pipeline(
            device,
            &slice_pipeline_layout,
            &shader,
            slice_entry.contract.entry_point,
        );

        let zero_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("RustTable bilateral zero group"),
            layout: &zero_layout,
            entries: &[
                named_buffer_entry(zero_entry.contract, "zero_grid", &grid_a)?,
                named_buffer_entry(zero_entry.contract, "params", &parameter_buffers[0])?,
            ],
        });
        let splat_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("RustTable bilateral splat group"),
            layout: &splat_layout,
            entries: &[
                named_buffer_entry(splat_entry.contract, "params", &parameter_buffers[1])?,
                named_buffer_entry(splat_entry.contract, "splat_input", &input)?,
                named_buffer_entry(splat_entry.contract, "splat_grid", &grid_a)?,
            ],
        });
        let horizontal_group = blur_group(
            device,
            &blur_layout,
            "RustTable bilateral x blur group",
            blur_entry.contract,
            &parameter_buffers[2],
            &grid_b,
            &grid_a,
        )?;
        let vertical_group = blur_group(
            device,
            &blur_layout,
            "RustTable bilateral y blur group",
            blur_entry.contract,
            &parameter_buffers[3],
            &grid_a,
            &grid_b,
        )?;
        let range_derivative_group = blur_group(
            device,
            &blur_layout,
            "RustTable bilateral z blur group",
            blur_z_entry.contract,
            &parameter_buffers[4],
            &grid_b,
            &grid_a,
        )?;
        let target_buffer = target.as_ref().unwrap_or(&input);
        let slice_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("RustTable bilateral slice group"),
            layout: &slice_layout,
            entries: &[
                named_buffer_entry(slice_entry.contract, "params", &parameter_buffers[5])?,
                named_buffer_entry(slice_entry.contract, "slice_input", &input)?,
                named_buffer_entry(slice_entry.contract, "slice_target", target_buffer)?,
                named_buffer_entry(slice_entry.contract, "slice_output", &output)?,
                named_buffer_entry(slice_entry.contract, "slice_grid", &grid_a)?,
            ],
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("RustTable bilateral encoder"),
        });
        dispatch(
            &mut encoder,
            "bilateral zero",
            &zero_pipeline,
            &zero_group,
            validated.dispatches[0].workgroups,
        );
        dispatch(
            &mut encoder,
            "bilateral splat",
            &splat_pipeline,
            &splat_group,
            validated.dispatches[1].workgroups,
        );
        encoder.copy_buffer_to_buffer(&grid_a, 0, &grid_b, 0, validated.grid_bytes);
        dispatch(
            &mut encoder,
            "bilateral blur x",
            &blur_pipeline,
            &horizontal_group,
            validated.dispatches[2].workgroups,
        );
        dispatch(
            &mut encoder,
            "bilateral blur y",
            &blur_pipeline,
            &vertical_group,
            validated.dispatches[3].workgroups,
        );
        dispatch(
            &mut encoder,
            "bilateral blur z",
            &blur_z_pipeline,
            &range_derivative_group,
            validated.dispatches[4].workgroups,
        );
        dispatch(
            &mut encoder,
            "bilateral slice",
            &slice_pipeline,
            &slice_group,
            validated.dispatches[5].workgroups,
        );
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, validated.pixel_bytes);
        if request.is_cancelled() {
            return Err(BilateralGridError::Cancelled);
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
            .map_err(|error| BilateralGridError::Poll(error.to_string()))?;
        receiver
            .recv()
            .map_err(|error| BilateralGridError::Readback(error.to_string()))?
            .map_err(BilateralGridError::Readback)?;
        let view = slice
            .get_mapped_range()
            .map_err(|error| BilateralGridError::Readback(error.to_string()))?;
        let (chunks, remainder) = view.as_chunks::<PIXEL_BYTES>();
        if !remainder.is_empty() {
            return Err(BilateralGridError::Readback(
                "mapped pixel buffer has a partial RGBA value".to_owned(),
            ));
        }
        let pixels = chunks.iter().map(pixel_from_bytes).collect::<Vec<_>>();
        drop(view);
        readback.unmap();
        if request.is_cancelled() {
            return Err(BilateralGridError::Cancelled);
        }
        if let Some((pixel, component)) = first_non_finite(&pixels) {
            return Err(BilateralGridError::NonFiniteOutput { pixel, component });
        }
        Ok(BilateralGridResult {
            pixels,
            dispatches: 6,
        })
    }
}

fn bilateral_layout(
    device: &wgpu::Device,
    label: &str,
    contract: &BilateralEntryContract,
    validated: &ValidatedRequest,
) -> Result<wgpu::BindGroupLayout, BilateralGridError> {
    let entries = contract
        .bindings
        .iter()
        .map(|binding| {
            let size = runtime_binding_size(binding, validated)?;
            if size < u64::from(binding.minimum_binding_size) {
                return Err(BilateralGridError::ShaderUnavailable);
            }
            let ty = if binding.storage {
                let read_only = match binding.access {
                    "read" => true,
                    "read_write" => false,
                    _ => return Err(BilateralGridError::ShaderUnavailable),
                };
                wgpu::BufferBindingType::Storage { read_only }
            } else if binding.access == "read" {
                wgpu::BufferBindingType::Uniform
            } else {
                return Err(BilateralGridError::ShaderUnavailable);
            };
            Ok(wgpu::BindGroupLayoutEntry {
                binding: binding.binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(size),
                },
                count: None,
            })
        })
        .collect::<Result<Vec<_>, BilateralGridError>>()?;
    Ok(
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries: &entries,
        }),
    )
}

fn runtime_binding_size(
    binding: &BindingContract,
    validated: &ValidatedRequest,
) -> Result<u64, BilateralGridError> {
    match binding.name {
        "params" => Ok(PARAMS_SIZE),
        "zero_grid" | "splat_grid" | "blur_input" | "blur_output" | "slice_grid" => {
            Ok(validated.grid_bytes)
        }
        "splat_input" | "slice_input" | "slice_target" | "slice_output" => {
            Ok(validated.pixel_bytes)
        }
        _ => Err(BilateralGridError::ShaderUnavailable),
    }
}

fn pipeline_layout(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::BindGroupLayout,
) -> wgpu::PipelineLayout {
    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    })
}

fn compute_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    entry_point: &str,
) -> wgpu::ComputePipeline {
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(entry_point),
        layout: Some(layout),
        module: shader,
        entry_point: Some(entry_point),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    })
}

fn named_buffer_entry<'a>(
    contract: &BilateralEntryContract,
    name: &str,
    buffer: &'a wgpu::Buffer,
) -> Result<wgpu::BindGroupEntry<'a>, BilateralGridError> {
    let binding = contract
        .bindings
        .iter()
        .find(|binding| binding.name == name)
        .ok_or(BilateralGridError::ShaderUnavailable)?;
    Ok(wgpu::BindGroupEntry {
        binding: binding.binding,
        resource: buffer.as_entire_binding(),
    })
}

fn blur_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    label: &str,
    contract: &BilateralEntryContract,
    params: &wgpu::Buffer,
    input: &wgpu::Buffer,
    output: &wgpu::Buffer,
) -> Result<wgpu::BindGroup, BilateralGridError> {
    Ok(device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            named_buffer_entry(contract, "params", params)?,
            named_buffer_entry(contract, "blur_input", input)?,
            named_buffer_entry(contract, "blur_output", output)?,
        ],
    }))
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
) -> Result<(), BilateralGridError> {
    let slice = buffer.slice(..);
    let mut view = slice
        .get_mapped_range_mut()
        .map_err(|error| BilateralGridError::Upload(error.to_string()))?;
    let expected_bytes = pixels
        .len()
        .checked_mul(PIXEL_BYTES)
        .ok_or(BilateralGridError::SizeOverflow)?;
    if view.len() != expected_bytes {
        return Err(BilateralGridError::Upload(
            "mapped upload buffer does not match RGBA pixel packing".to_owned(),
        ));
    }
    for (pixel_index, pixel) in pixels.iter().enumerate() {
        let mut bytes = [0_u8; PIXEL_BYTES];
        let (component_bytes, remainder) = bytes.as_chunks_mut::<4>();
        debug_assert!(remainder.is_empty());
        for (component, destination) in pixel.iter().zip(component_bytes) {
            destination.copy_from_slice(&component.to_le_bytes());
        }
        let start = pixel_index
            .checked_mul(PIXEL_BYTES)
            .ok_or(BilateralGridError::SizeOverflow)?;
        let end = start
            .checked_add(PIXEL_BYTES)
            .ok_or(BilateralGridError::SizeOverflow)?;
        view.slice(start..end).copy_from_slice(&bytes);
    }
    drop(view);
    buffer.unmap();
    Ok(())
}

fn write_mapped_bytes(buffer: &wgpu::Buffer, bytes: &[u8]) -> Result<(), BilateralGridError> {
    let slice = buffer.slice(..);
    let mut view = slice
        .get_mapped_range_mut()
        .map_err(|error| BilateralGridError::Upload(error.to_string()))?;
    if view.len() != bytes.len() {
        return Err(BilateralGridError::Upload(
            "mapped upload buffer does not match payload length".to_owned(),
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

fn pixel_from_bytes(bytes: &[u8; PIXEL_BYTES]) -> [f32; 4] {
    let [
        l0,
        l1,
        l2,
        l3,
        a0,
        a1,
        a2,
        a3,
        b0,
        b1,
        b2,
        b3,
        alpha0,
        alpha1,
        alpha2,
        alpha3,
    ] = *bytes;
    [
        f32::from_le_bytes([l0, l1, l2, l3]),
        f32::from_le_bytes([a0, a1, a2, a3]),
        f32::from_le_bytes([b0, b1, b2, b3]),
        f32::from_le_bytes([alpha0, alpha1, alpha2, alpha3]),
    ]
}

fn pack_params(validated: &ValidatedRequest, dispatch: DispatchPlan) -> [u8; 64] {
    let words = [
        validated.width,
        validated.height,
        validated.grid_dimensions[0],
        validated.grid_dimensions[1],
        validated.grid_dimensions[2],
        validated.effective_sigma_s.to_bits(),
        validated.effective_sigma_r.to_bits(),
        validated.detail.to_bits(),
        dispatch.strides[0],
        dispatch.strides[1],
        dispatch.strides[2],
        dispatch.sizes[0],
        dispatch.sizes[1],
        dispatch.sizes[2],
        validated.pixel_count,
        validated.grid_values,
    ];
    let mut bytes = [0_u8; 64];
    let (chunks, remainder) = bytes.as_chunks_mut::<4>();
    debug_assert!(remainder.is_empty());
    for (chunk, word) in chunks.iter_mut().zip(words) {
        chunk.copy_from_slice(&word.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss
    )]

    use super::*;

    const SHADER: &str = include_str!("../shaders/bilateral.wgsl");
    const SMALL_WIDTH: usize = 3;
    const SMALL_HEIGHT: usize = 2;
    const SMALL_GRID: [usize; 3] = [5, 4, 5];
    const SMALL_SIGMA_S: f32 = 0.75;
    const SMALL_SIGMA_R: f32 = 25.0;
    const TEST_TRANSIENT_MEMORY_BUDGET: u64 = 512 * 1024 * 1024;

    fn pixels() -> Vec<[f32; 4]> {
        vec![
            [10.0, -15.0, 22.0, 0.10],
            [24.0, 18.0, -9.0, 0.20],
            [41.0, 5.0, 13.0, 0.30],
            [57.0, -4.0, -21.0, 0.40],
            [73.0, 27.0, 2.0, 0.50],
            [91.0, -31.0, 7.0, 0.60],
        ]
    }

    fn assert_close(actual: f32, expected: f32) {
        let tolerance = 0.000_25 * expected.abs().max(1.0);
        assert!(
            (actual - expected).abs() <= tolerance,
            "expected {expected}, got {actual} (tolerance {tolerance})"
        );
    }

    fn reference_blur(
        input: &[f32],
        strides: [usize; 3],
        sizes: [usize; 3],
        derivative: bool,
    ) -> Vec<f32> {
        let mut output = vec![0.0; input.len()];
        for outer in 0..sizes[0] {
            for line in 0..sizes[1] {
                let base = outer * strides[0] + line * strides[1];
                for position in 0..sizes[2] {
                    let sample = |relative: isize| {
                        let sample_position = match relative {
                            -2 => position.checked_sub(2),
                            -1 => position.checked_sub(1),
                            0 => Some(position),
                            1 => position.checked_add(1),
                            2 => position.checked_add(2),
                            _ => None,
                        }
                        .filter(|sample_position| *sample_position < sizes[2]);
                        sample_position.map_or(0.0, |sample_position| {
                            input[base + sample_position * strides[2]]
                        })
                    };
                    let index = base + position * strides[2];
                    output[index] = if derivative {
                        (4.0 * (sample(1) - sample(-1)) + 2.0 * (sample(2) - sample(-2))) / 16.0
                    } else {
                        (6.0 * sample(0) + 4.0 * (sample(-1) + sample(1)) + sample(-2) + sample(2))
                            / 16.0
                    };
                }
            }
        }
        output
    }

    fn reference_slice(
        input: &[[f32; 4]],
        width: usize,
        height: usize,
        dimensions: [usize; 3],
        sigma_s: f32,
        sigma_r: f32,
        detail: f32,
    ) -> Vec<[f32; 4]> {
        let [size_x, size_y, size_z] = dimensions;
        let xy = size_x * size_y;
        let mut grid = vec![0.0; xy * size_z];
        for y in 0..height {
            for x in 0..width {
                let lightness = input[y * width + x][0];
                let grid_x = (x as f32 / sigma_s).clamp(0.0, (size_x - 1) as f32);
                let grid_y = (y as f32 / sigma_s).clamp(0.0, (size_y - 1) as f32);
                let grid_z = (lightness / sigma_r).clamp(0.0, (size_z - 1) as f32);
                let base_x = (grid_x as usize).min(size_x - 2);
                let base_y = (grid_y as usize).min(size_y - 2);
                let base_z = (grid_z as usize).min(size_z - 2);
                let fractions = [
                    grid_x - base_x as f32,
                    grid_y - base_y as f32,
                    grid_z - base_z as f32,
                ];
                let base = base_x + size_x * (base_y + size_y * base_z);
                let scale = 100.0 / (sigma_s * sigma_s);
                for z_offset in 0..=1 {
                    for y_offset in 0..=1 {
                        for x_offset in 0..=1 {
                            let weight_x = if x_offset == 0 {
                                1.0 - fractions[0]
                            } else {
                                fractions[0]
                            };
                            let weight_y = if y_offset == 0 {
                                1.0 - fractions[1]
                            } else {
                                fractions[1]
                            };
                            let weight_z = if z_offset == 0 {
                                1.0 - fractions[2]
                            } else {
                                fractions[2]
                            };
                            let offset = x_offset + size_x * (y_offset + size_y * z_offset);
                            grid[base + offset] += scale * weight_x * weight_y * weight_z;
                        }
                    }
                }
            }
        }
        let grid = reference_blur(&grid, [xy, size_x, 1], [size_z, size_y, size_x], false);
        let grid = reference_blur(&grid, [xy, 1, size_x], [size_z, size_x, size_y], false);
        let grid = reference_blur(&grid, [1, size_x, xy], [size_x, size_y, size_z], true);

        input
            .iter()
            .enumerate()
            .map(|(pixel_index, source)| {
                let x = pixel_index % width;
                let y = pixel_index / width;
                let grid_x = (x as f32 / sigma_s).clamp(0.0, (size_x - 1) as f32);
                let grid_y = (y as f32 / sigma_s).clamp(0.0, (size_y - 1) as f32);
                let grid_z = (source[0] / sigma_r).clamp(0.0, (size_z - 1) as f32);
                let base_x = (grid_x as usize).min(size_x - 2);
                let base_y = (grid_y as usize).min(size_y - 2);
                let base_z = (grid_z as usize).min(size_z - 2);
                let fractions = [
                    grid_x - base_x as f32,
                    grid_y - base_y as f32,
                    grid_z - base_z as f32,
                ];
                let base = base_x + size_x * (base_y + size_y * base_z);
                let mut interpolation = 0.0;
                for z_offset in 0..=1 {
                    for y_offset in 0..=1 {
                        for x_offset in 0..=1 {
                            let weight_x = if x_offset == 0 {
                                1.0 - fractions[0]
                            } else {
                                fractions[0]
                            };
                            let weight_y = if y_offset == 0 {
                                1.0 - fractions[1]
                            } else {
                                fractions[1]
                            };
                            let weight_z = if z_offset == 0 {
                                1.0 - fractions[2]
                            } else {
                                fractions[2]
                            };
                            let offset = x_offset + size_x * (y_offset + size_y * z_offset);
                            interpolation += grid[base + offset] * weight_x * weight_y * weight_z;
                        }
                    }
                }
                let mut output = *source;
                output[0] = (source[0] - detail * sigma_r * 0.04 * interpolation).max(0.0);
                output
            })
            .collect()
    }

    #[test]
    fn checked_shader_has_the_six_retained_entry_points() {
        let module = naga::front::wgsl::parse_str(SHADER).expect("bilateral WGSL syntax");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("bilateral WGSL validation");
        let mut entries = module
            .entry_points
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();
        entries.sort_unstable();
        assert_eq!(
            entries,
            [
                "blur_line",
                "blur_line_z",
                "slice",
                "slice_to_output",
                "splat",
                "zero",
            ]
        );
        let splat = module
            .entry_points
            .iter()
            .find(|entry| entry.name == "splat")
            .expect("splat entry");
        assert_eq!(splat.workgroup_size, [16, 16, 1]);
    }

    #[test]
    fn runtime_program_uses_checked_registry_source_and_contracts() {
        let program = BilateralProgram::checked().expect("checked bilateral program");
        assert_eq!(program.source, SHADER);
        for runtime_entry in program.entries {
            assert_eq!(
                runtime_entry.shader.reflection.workgroup_size,
                runtime_entry.contract.workgroup_size
            );
            assert_eq!(
                runtime_entry.shader.reflection.bindings.len(),
                runtime_entry.contract.bindings.len()
            );
            for (reflection, contract) in runtime_entry
                .shader
                .reflection
                .bindings
                .iter()
                .zip(runtime_entry.contract.bindings)
            {
                assert_eq!(reflection.binding, contract.binding);
                assert_eq!(reflection.name, contract.name);
                assert_eq!(reflection.access, contract.access);
                assert_eq!(
                    reflection.minimum_binding_size,
                    contract.minimum_binding_size
                );
                assert_eq!(reflection.type_description, contract.type_description);
            }
        }
    }

    #[test]
    fn resolved_geometry_drives_exact_gpu_memory_and_dispatch_plan() {
        let input = vec![[50.0, 0.0, 0.0, 1.0]; 8 * 6];
        let request = BilateralGridRequest::slice(
            &input,
            8,
            6,
            [17, 13, 5],
            0.5,
            25.0,
            -1.0,
            TEST_TRANSIENT_MEMORY_BUDGET,
        );
        let validated =
            ValidatedRequest::new(request, &wgpu::Limits::downlevel_defaults()).expect("request");
        assert_eq!(validated.grid_bytes, 4_420);
        assert_eq!(validated.dispatches.len(), 6);
        assert_eq!(validated.dispatches[2].strides, [17 * 13, 17, 1]);
        assert_eq!(validated.dispatches[3].strides, [17 * 13, 1, 17]);
        assert_eq!(validated.dispatches[4].strides, [1, 17, 17 * 13]);
    }

    #[test]
    fn aggregate_admission_counts_every_explicit_allocation_at_the_exact_boundary() {
        let input = [[50.0, 0.0, 0.0, 1.0]];
        let target = [[75.0, 1.0, 2.0, 0.5]];
        let limits = wgpu::Limits::downlevel_defaults();
        let pixel_bytes = 16;
        let grid_bytes = 2 * 2 * 2 * 4;

        // Retained `dt_bilateral_slice_cl` needs the two grid buffers while
        // guide, output, and readback are simultaneously live in this WGPU
        // boundary. Six dispatch parameter buffers add 6 * 64 bytes, and the
        // returned host pixels add one more pixel-sized allocation.
        let replace_required =
            aggregate_transient_bytes(pixel_bytes, grid_bytes, BilateralSlice::Replace)
                .expect("replace bytes");
        assert_eq!(
            replace_required,
            4 * pixel_bytes + 2 * grid_bytes + 6 * PARAMS_SIZE
        );
        assert!(matches!(
            ValidatedRequest::new(
                BilateralGridRequest::slice(
                    &input,
                    1,
                    1,
                    [2, 2, 2],
                    1.0,
                    25.0,
                    -1.0,
                    replace_required - 1,
                ),
                &limits,
            ),
            Err(BilateralGridError::AggregateMemoryLimit {
                required,
                limit
            }) if required == replace_required && limit == replace_required - 1
        ));
        ValidatedRequest::new(
            BilateralGridRequest::slice(&input, 1, 1, [2, 2, 2], 1.0, 25.0, -1.0, replace_required),
            &limits,
        )
        .expect("exact replace budget");

        // Retained `dt_bilateral_memory_use2` adds one full image for
        // slice-to-output. The safe WGPU path represents that lifetime as its
        // fourth pixel-sized device buffer, in addition to the returned host
        // pixels.
        let additive_required =
            aggregate_transient_bytes(pixel_bytes, grid_bytes, BilateralSlice::AddTo(&target))
                .expect("additive bytes");
        assert_eq!(
            additive_required,
            5 * pixel_bytes + 2 * grid_bytes + 6 * PARAMS_SIZE
        );
        assert!(matches!(
            ValidatedRequest::new(
                BilateralGridRequest::slice_to_output(
                    &input,
                    &target,
                    1,
                    1,
                    [2, 2, 2],
                    1.0,
                    25.0,
                    -1.0,
                    additive_required - 1,
                ),
                &limits,
            ),
            Err(BilateralGridError::AggregateMemoryLimit {
                required,
                limit
            }) if required == additive_required && limit == additive_required - 1
        ));
        ValidatedRequest::new(
            BilateralGridRequest::slice_to_output(
                &input,
                &target,
                1,
                1,
                [2, 2, 2],
                1.0,
                25.0,
                -1.0,
                additive_required,
            ),
            &limits,
        )
        .expect("exact additive budget");
    }

    #[test]
    fn aggregate_admission_requires_a_nonzero_budget_and_checked_arithmetic() {
        let input = [[50.0, 0.0, 0.0, 1.0]];
        assert!(matches!(
            ValidatedRequest::new(
                BilateralGridRequest::slice(&input, 1, 1, [2, 2, 2], 1.0, 25.0, -1.0, 0,),
                &wgpu::Limits::downlevel_defaults(),
            ),
            Err(BilateralGridError::InvalidMemoryBudget)
        ));
        assert_eq!(
            aggregate_transient_bytes(u64::MAX, 1, BilateralSlice::Replace),
            Err(BilateralGridError::SizeOverflow)
        );
    }

    #[test]
    fn validation_rejects_shape_nonfinite_detail_and_target() {
        let limits = wgpu::Limits::downlevel_defaults();
        let short = [[1.0, 2.0, 3.0, 4.0]; 5];
        assert!(matches!(
            ValidatedRequest::new(
                BilateralGridRequest::slice(
                    &short,
                    SMALL_WIDTH,
                    SMALL_HEIGHT,
                    SMALL_GRID,
                    SMALL_SIGMA_S,
                    SMALL_SIGMA_R,
                    -1.0,
                    TEST_TRANSIENT_MEMORY_BUDGET
                ),
                &limits
            ),
            Err(BilateralGridError::BufferShape { .. })
        ));
        let mut invalid = pixels();
        invalid[2][0] = f32::NAN;
        assert!(matches!(
            ValidatedRequest::new(
                BilateralGridRequest::slice(
                    &invalid,
                    SMALL_WIDTH,
                    SMALL_HEIGHT,
                    SMALL_GRID,
                    SMALL_SIGMA_S,
                    SMALL_SIGMA_R,
                    -1.0,
                    TEST_TRANSIENT_MEMORY_BUDGET
                ),
                &limits
            ),
            Err(BilateralGridError::NonFiniteInput {
                pixel: 2,
                component: 0
            })
        ));
        let mut invalid_component = pixels();
        invalid_component[1][3] = f32::INFINITY;
        assert!(matches!(
            ValidatedRequest::new(
                BilateralGridRequest::slice(
                    &invalid_component,
                    SMALL_WIDTH,
                    SMALL_HEIGHT,
                    SMALL_GRID,
                    SMALL_SIGMA_S,
                    SMALL_SIGMA_R,
                    -1.0,
                    TEST_TRANSIENT_MEMORY_BUDGET
                ),
                &limits
            ),
            Err(BilateralGridError::NonFiniteInput {
                pixel: 1,
                component: 3
            })
        ));
        assert!(matches!(
            ValidatedRequest::new(
                BilateralGridRequest::slice(
                    &pixels(),
                    SMALL_WIDTH,
                    SMALL_HEIGHT,
                    SMALL_GRID,
                    SMALL_SIGMA_S,
                    SMALL_SIGMA_R,
                    f32::NAN,
                    TEST_TRANSIENT_MEMORY_BUDGET
                ),
                &limits
            ),
            Err(BilateralGridError::NonFiniteDetail)
        ));
        assert!(matches!(
            ValidatedRequest::new(
                BilateralGridRequest::slice_to_output(
                    &pixels(),
                    &short,
                    SMALL_WIDTH,
                    SMALL_HEIGHT,
                    SMALL_GRID,
                    SMALL_SIGMA_S,
                    SMALL_SIGMA_R,
                    -1.0,
                    TEST_TRANSIENT_MEMORY_BUDGET
                ),
                &limits
            ),
            Err(BilateralGridError::TargetShape { .. })
        ));
        let mut invalid_target = pixels();
        invalid_target[4][2] = f32::NEG_INFINITY;
        assert!(matches!(
            ValidatedRequest::new(
                BilateralGridRequest::slice_to_output(
                    &pixels(),
                    &invalid_target,
                    SMALL_WIDTH,
                    SMALL_HEIGHT,
                    SMALL_GRID,
                    SMALL_SIGMA_S,
                    SMALL_SIGMA_R,
                    -1.0,
                    TEST_TRANSIENT_MEMORY_BUDGET
                ),
                &limits
            ),
            Err(BilateralGridError::NonFiniteTarget {
                pixel: 4,
                component: 2
            })
        ));
    }

    #[test]
    fn asymmetric_short_axes_remain_plannable() {
        let input = vec![[25.0, 1.0, 2.0, 0.5]; 10_000];
        let request = BilateralGridRequest::slice(
            &input,
            10_000,
            1,
            [3_001, 2, 5],
            10_000.0 / 3_000.0,
            25.0,
            -1.0,
            TEST_TRANSIENT_MEMORY_BUDGET,
        );
        ValidatedRequest::new(request, &wgpu::Limits::downlevel_defaults())
            .expect("short-axis-safe plan");
    }

    #[test]
    fn native_two_dimensional_zero_shape_accepts_a_buffer_fitting_large_grid() {
        let width = 1_024;
        let height = 1_024;
        let input = vec![[50.0, 0.0, 0.0, 1.0]; width * height];
        let request = BilateralGridRequest::slice(
            &input,
            width,
            height,
            [2_049, 2_049, 5],
            0.5,
            25.0,
            -1.0,
            TEST_TRANSIENT_MEMORY_BUDGET,
        );
        let validated =
            ValidatedRequest::new(request, &wgpu::Limits::downlevel_defaults()).expect("request");
        assert_eq!(
            validated.dispatches[0].workgroups,
            [2_049_u32.div_ceil(16), (2_049_u32 * 5).div_ceil(16), 1]
        );
    }

    #[test]
    fn derived_shader_arithmetic_must_remain_finite() {
        let input = [[50.0, 0.0, 0.0, 1.0]];
        let limits = wgpu::Limits::downlevel_defaults();
        assert!(matches!(
            ValidatedRequest::new(
                BilateralGridRequest::slice(
                    &input,
                    1,
                    1,
                    [2, 2, 2],
                    f32::MIN_POSITIVE,
                    25.0,
                    -1.0,
                    TEST_TRANSIENT_MEMORY_BUDGET,
                ),
                &limits
            ),
            Err(BilateralGridError::InvalidGeometry)
        ));
        assert!(matches!(
            ValidatedRequest::new(
                BilateralGridRequest::slice(
                    &input,
                    1,
                    1,
                    [2, 2, 2],
                    1.0,
                    25.0,
                    f32::MAX,
                    TEST_TRANSIENT_MEMORY_BUDGET,
                ),
                &limits
            ),
            Err(BilateralGridError::NonFiniteDetail)
        ));
    }

    #[test]
    fn sparse_binding_indices_require_the_highest_registered_slot() {
        let input = [[50.0, 0.0, 0.0, 1.0]];
        let mut limits = wgpu::Limits::downlevel_defaults();
        limits.max_bindings_per_bind_group = 9;
        assert!(matches!(
            ValidatedRequest::new(
                BilateralGridRequest::slice(
                    &input,
                    1,
                    1,
                    [2, 2, 2],
                    1.0,
                    25.0,
                    -1.0,
                    TEST_TRANSIENT_MEMORY_BUDGET,
                ),
                &limits
            ),
            Err(BilateralGridError::InsufficientComputeLimits)
        ));
        limits.max_bindings_per_bind_group = 10;
        ValidatedRequest::new(
            BilateralGridRequest::slice(
                &input,
                1,
                1,
                [2, 2, 2],
                1.0,
                25.0,
                -1.0,
                TEST_TRANSIENT_MEMORY_BUDGET,
            ),
            &limits,
        )
        .expect("binding slots through index nine are available");
    }

    #[test]
    fn cancellation_is_attached_without_changing_the_resolved_geometry() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let input = pixels();
        let request = BilateralGridRequest::slice(
            &input,
            SMALL_WIDTH,
            SMALL_HEIGHT,
            SMALL_GRID,
            SMALL_SIGMA_S,
            SMALL_SIGMA_R,
            -1.0,
            TEST_TRANSIENT_MEMORY_BUDGET,
        )
        .with_cancellation(&cancellation);
        assert!(request.is_cancelled());
        ValidatedRequest::new(request, &wgpu::Limits::downlevel_defaults())
            .expect("geometry remains valid");
    }

    #[tokio::test]
    async fn gpu_slice_and_additive_slice_match_the_cpu_reference_when_available() {
        let Ok(runtime) = crate::GpuRuntime::initialize(crate::GpuRuntimeConfig::default()).await
        else {
            return;
        };
        if runtime.is_cpu_only() {
            return;
        }
        let input = pixels();
        let expected = reference_slice(
            &input,
            SMALL_WIDTH,
            SMALL_HEIGHT,
            SMALL_GRID,
            SMALL_SIGMA_S,
            SMALL_SIGMA_R,
            -1.0,
        );
        let actual = runtime
            .execute_bilateral_grid(BilateralGridRequest::slice(
                &input,
                SMALL_WIDTH,
                SMALL_HEIGHT,
                SMALL_GRID,
                SMALL_SIGMA_S,
                SMALL_SIGMA_R,
                -1.0,
                TEST_TRANSIENT_MEMORY_BUDGET,
            ))
            .expect("GPU slice");
        assert_eq!(actual.dispatches(), 6);
        for ((actual, expected), source) in actual.pixels().iter().zip(&expected).zip(input.iter())
        {
            assert_close(actual[0], expected[0]);
            assert_eq!(actual[1..], source[1..]);
        }

        let target = vec![[120.0, 11.0, 12.0, 0.75]; input.len()];
        let additive = runtime
            .execute_bilateral_grid(BilateralGridRequest::slice_to_output(
                &input,
                &target,
                SMALL_WIDTH,
                SMALL_HEIGHT,
                SMALL_GRID,
                SMALL_SIGMA_S,
                SMALL_SIGMA_R,
                -1.0,
                TEST_TRANSIENT_MEMORY_BUDGET,
            ))
            .expect("GPU additive slice");
        for (((actual, target), source), filtered) in additive
            .pixels()
            .iter()
            .zip(&target)
            .zip(&input)
            .zip(expected)
        {
            assert_close(actual[0], (target[0] + filtered[0] - source[0]).max(0.0));
            assert_eq!(actual[1..], target[1..]);
        }
    }
}
