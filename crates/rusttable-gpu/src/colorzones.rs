//! Safe dedicated WGPU execution for Darktable
//! `data/kernels/basic.cl::{colorzones_v3,colorzones}` and the matching
//! `src/iop/colorzones.c::process_cl` dispatch boundary.

use std::fmt;
use std::num::NonZeroU64;

use crate::{CancellationToken, FaultState, GpuRuntime};

/// Native Color Zones LUT resolution (`DT_IOP_COLORZONES_LUT_RES`).
pub const COLORZONES_LUT_RESOLUTION: usize = 65_536;

const WORKGROUP_SIZE: u32 = 256;
const PIXEL_BYTES: usize = 16;
const PIXEL_BYTES_U64: u64 = 16;
const PARAMS_SIZE: u64 = 16;
const PARAMS_BYTES: usize = 16;
const LUT_BYTES: u64 = 65_536 * 4;
const STORAGE_BINDING_COUNT: u32 = 4;
const BINDING_COUNT: u32 = 5;
const COLORZONES_SHADER_SOURCE: &str = include_str!("../shaders/colorzones.wgsl");

/// Returns the Color Zones leaf's complete transient allocation for a pixel count.
///
/// The total covers the read-write device pixel buffer, readback buffer, returned
/// host pixels, three immutable LUT buffers, and the parameter uniform. Pixelpipe
/// uses this exact resource estimate as its per-dispatch admission budget.
#[must_use]
pub fn colorzones_transient_memory_bytes(pixel_count: usize) -> Option<u64> {
    u64::try_from(pixel_count)
        .ok()?
        .checked_mul(PIXEL_BYTES_U64)
        .and_then(aggregate_transient_bytes)
}

/// Native channel used to select a Color Zones LUT coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ColorZonesSelection {
    Lightness = 0,
    Chroma = 1,
    Hue = 2,
}

impl ColorZonesSelection {
    const fn raw(self) -> u32 {
        match self {
            Self::Lightness => 0,
            Self::Chroma => 1,
            Self::Hue => 2,
        }
    }
}

/// Native Color Zones point-processing branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ColorZonesMode {
    /// `basic.cl::colorzones_v3`.
    Smooth = 0,
    /// `basic.cl::colorzones`.
    Strong = 1,
}

impl ColorZonesMode {
    const fn raw(self) -> u32 {
        match self {
            Self::Smooth => 0,
            Self::Strong => 1,
        }
    }
}

/// Caller-provided identity of the immutable snapshot represented by a request.
///
/// The executor returns the same identity only after cancellation checks around
/// submission and readback, allowing pixelpipe to reject stale publication
/// without re-deriving identity from the large LUT resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColorZonesRequestIdentity([u8; 32]);

impl ColorZonesRequestIdentity {
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

/// A checked Color Zones WGPU request over Darktable-scale D50 Lab pixels.
///
/// The three LUTs are immutable, separate resources and must each contain
/// exactly 65,536 finite entries. `transient_memory_budget_bytes` bounds every
/// device buffer allocated by this execution plus its returned host pixel
/// vector.
#[derive(Debug, Clone, Copy)]
pub struct ColorZonesRequest<'a> {
    pixels: &'a [[f32; 4]],
    lightness_lut: &'a [f32],
    chroma_lut: &'a [f32],
    hue_lut: &'a [f32],
    selection: ColorZonesSelection,
    mode: ColorZonesMode,
    identity: ColorZonesRequestIdentity,
    transient_memory_budget_bytes: u64,
    cancellation: Option<&'a CancellationToken>,
}

impl<'a> ColorZonesRequest<'a> {
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "request fields preserve the native Color Zones LUT and dispatch ABI"
    )]
    pub const fn new(
        pixels: &'a [[f32; 4]],
        lightness_lut: &'a [f32],
        chroma_lut: &'a [f32],
        hue_lut: &'a [f32],
        selection: ColorZonesSelection,
        mode: ColorZonesMode,
        identity: ColorZonesRequestIdentity,
        transient_memory_budget_bytes: u64,
    ) -> Self {
        Self {
            pixels,
            lightness_lut,
            chroma_lut,
            hue_lut,
            selection,
            mode,
            identity,
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

    #[must_use]
    pub const fn identity(self) -> ColorZonesRequestIdentity {
        self.identity
    }

    fn is_cancelled(self) -> bool {
        self.cancellation
            .is_some_and(CancellationToken::is_cancelled)
    }

    const fn luts(self) -> [(ColorZonesLutResource, &'a [f32]); 3] {
        [
            (ColorZonesLutResource::Lightness, self.lightness_lut),
            (ColorZonesLutResource::Chroma, self.chroma_lut),
            (ColorZonesLutResource::Hue, self.hue_lut),
        ]
    }
}

/// Read-back from one Color Zones dispatch.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorZonesResult {
    pixels: Vec<[f32; 4]>,
    identity: ColorZonesRequestIdentity,
    dispatches: u32,
}

impl ColorZonesResult {
    #[must_use]
    pub fn pixels(&self) -> &[[f32; 4]] {
        &self.pixels
    }

    #[must_use]
    pub fn into_pixels(self) -> Vec<[f32; 4]> {
        self.pixels
    }

    #[must_use]
    pub const fn identity(&self) -> ColorZonesRequestIdentity {
        self.identity
    }

    #[must_use]
    pub const fn dispatches(&self) -> u32 {
        self.dispatches
    }
}

/// Identifies one immutable native Color Zones LUT resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorZonesLutResource {
    Lightness,
    Chroma,
    Hue,
}

impl fmt::Display for ColorZonesLutResource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Lightness => "lightness",
            Self::Chroma => "chroma",
            Self::Hue => "hue",
        })
    }
}

/// Checked failure from the dedicated Color Zones WGPU path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorZonesError {
    CpuOnly,
    Unhealthy,
    EmptyInput,
    TooManyPixels,
    NonFiniteInput {
        pixel: usize,
        component: usize,
    },
    InvalidLut {
        resource: ColorZonesLutResource,
        expected: usize,
        actual: usize,
    },
    NonFiniteLut {
        resource: ColorZonesLutResource,
        index: usize,
    },
    InvalidMemoryBudget,
    SizeOverflow,
    BufferLimit {
        resource: &'static str,
        required: u64,
        limit: u64,
    },
    AggregateMemoryLimit {
        required: u64,
        limit: u64,
    },
    InsufficientComputeLimits,
    TooManyWorkgroups,
    ShaderUnavailable,
    Cancelled,
    Upload(String),
    Poll(String),
    Readback(String),
}

impl fmt::Display for ColorZonesError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CpuOnly => formatter.write_str("Color Zones WGPU execution is CPU-only"),
            Self::Unhealthy => formatter.write_str("Color Zones WGPU execution is unhealthy"),
            Self::EmptyInput => formatter.write_str("Color Zones WGPU input is empty"),
            Self::TooManyPixels => formatter.write_str("Color Zones WGPU input exceeds u32 pixels"),
            Self::NonFiniteInput { pixel, component } => write!(
                formatter,
                "Color Zones WGPU input component {component} at pixel {pixel} is non-finite"
            ),
            Self::InvalidLut {
                resource,
                expected,
                actual,
            } => write!(
                formatter,
                "Color Zones WGPU {resource} LUT has {actual} entries; expected {expected}"
            ),
            Self::NonFiniteLut { resource, index } => write!(
                formatter,
                "Color Zones WGPU {resource} LUT entry {index} is non-finite"
            ),
            Self::InvalidMemoryBudget => {
                formatter.write_str("Color Zones WGPU transient-memory budget must be nonzero")
            }
            Self::SizeOverflow => formatter.write_str("Color Zones WGPU size overflowed"),
            Self::BufferLimit {
                resource,
                required,
                limit,
            } => write!(
                formatter,
                "Color Zones WGPU {resource} requires {required} bytes; device limit is {limit}"
            ),
            Self::AggregateMemoryLimit { required, limit } => write!(
                formatter,
                "Color Zones WGPU transient allocations require {required} bytes; transient-memory budget is {limit}"
            ),
            Self::InsufficientComputeLimits => formatter
                .write_str("Color Zones WGPU device compute or binding limits are insufficient"),
            Self::TooManyWorkgroups => {
                formatter.write_str("Color Zones WGPU dispatch exceeds the device workgroup limit")
            }
            Self::ShaderUnavailable => {
                formatter.write_str("Color Zones WGPU shader is unavailable")
            }
            Self::Cancelled => formatter.write_str("Color Zones WGPU execution was cancelled"),
            Self::Upload(error) => write!(formatter, "Color Zones WGPU upload failed: {error}"),
            Self::Poll(error) => write!(formatter, "Color Zones WGPU poll failed: {error}"),
            Self::Readback(error) => write!(formatter, "Color Zones WGPU readback failed: {error}"),
        }
    }
}

impl std::error::Error for ColorZonesError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ValidatedRequest {
    pixel_count: u32,
    pixel_bytes: u64,
    workgroups: u32,
    aggregate_transient_bytes: u64,
}

impl ValidatedRequest {
    fn new(request: ColorZonesRequest<'_>, limits: &wgpu::Limits) -> Result<Self, ColorZonesError> {
        if request.pixels.is_empty() {
            return Err(ColorZonesError::EmptyInput);
        }
        let pixel_count =
            u32::try_from(request.pixels.len()).map_err(|_| ColorZonesError::TooManyPixels)?;
        if let Some((pixel, component)) = first_non_finite(request.pixels) {
            return Err(ColorZonesError::NonFiniteInput { pixel, component });
        }
        for (resource, lut) in request.luts() {
            if lut.len() != COLORZONES_LUT_RESOLUTION {
                return Err(ColorZonesError::InvalidLut {
                    resource,
                    expected: COLORZONES_LUT_RESOLUTION,
                    actual: lut.len(),
                });
            }
            if let Some(index) = lut.iter().position(|value| !value.is_finite()) {
                return Err(ColorZonesError::NonFiniteLut { resource, index });
            }
        }
        if request.transient_memory_budget_bytes == 0 {
            return Err(ColorZonesError::InvalidMemoryBudget);
        }

        let pixel_bytes = u64::from(pixel_count)
            .checked_mul(PIXEL_BYTES_U64)
            .ok_or(ColorZonesError::SizeOverflow)?;
        let storage_limit = limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size);
        for (resource, required) in [("pixel buffer", pixel_bytes), ("LUT buffer", LUT_BYTES)] {
            if required > storage_limit {
                return Err(ColorZonesError::BufferLimit {
                    resource,
                    required,
                    limit: storage_limit,
                });
            }
        }
        let uniform_limit = limits
            .max_uniform_buffer_binding_size
            .min(limits.max_buffer_size);
        if PARAMS_SIZE > uniform_limit {
            return Err(ColorZonesError::BufferLimit {
                resource: "parameter buffer",
                required: PARAMS_SIZE,
                limit: uniform_limit,
            });
        }
        if limits.max_bind_groups < 1
            || limits.max_bindings_per_bind_group < BINDING_COUNT
            || limits.max_storage_buffers_per_shader_stage < STORAGE_BINDING_COUNT
            || limits.max_uniform_buffers_per_shader_stage < 1
            || limits.max_compute_invocations_per_workgroup < WORKGROUP_SIZE
            || limits.max_compute_workgroup_size_x < WORKGROUP_SIZE
            || limits.max_compute_workgroup_size_y < 1
            || limits.max_compute_workgroup_size_z < 1
        {
            return Err(ColorZonesError::InsufficientComputeLimits);
        }
        let workgroups = pixel_count.div_ceil(WORKGROUP_SIZE);
        if workgroups > limits.max_compute_workgroups_per_dimension {
            return Err(ColorZonesError::TooManyWorkgroups);
        }
        let aggregate_transient_bytes =
            aggregate_transient_bytes(pixel_bytes).ok_or(ColorZonesError::SizeOverflow)?;
        if aggregate_transient_bytes > request.transient_memory_budget_bytes {
            return Err(ColorZonesError::AggregateMemoryLimit {
                required: aggregate_transient_bytes,
                limit: request.transient_memory_budget_bytes,
            });
        }

        Ok(Self {
            pixel_count,
            pixel_bytes,
            workgroups,
            aggregate_transient_bytes,
        })
    }
}

impl GpuRuntime {
    /// Dispatches one checked Color Zones Smooth or Strong operation.
    ///
    /// The caller must provide Darktable-scale D50 Lab plus alpha. This leaf
    /// deliberately does not perform colorspace conversion, masks, blending, or
    /// publication; those remain typed pixelpipe responsibilities.
    #[expect(
        clippy::too_many_lines,
        reason = "the GPU path keeps LUT uploads, bind-group layout, dispatch, and readback in native order"
    )]
    pub fn execute_colorzones(
        &self,
        request: ColorZonesRequest<'_>,
    ) -> Result<ColorZonesResult, ColorZonesError> {
        if request.is_cancelled() {
            return Err(ColorZonesError::Cancelled);
        }
        if self.is_cpu_only() {
            return Err(ColorZonesError::CpuOnly);
        }
        if !matches!(
            self.snapshot().state,
            FaultState::Healthy | FaultState::Degraded
        ) {
            return Err(ColorZonesError::Unhealthy);
        }
        let (device, queue) = self.handles().ok_or(ColorZonesError::CpuOnly)?;
        if COLORZONES_SHADER_SOURCE.is_empty() {
            return Err(ColorZonesError::ShaderUnavailable);
        }
        let validated = ValidatedRequest::new(request, &device.limits())?;
        if request.is_cancelled() {
            return Err(ColorZonesError::Cancelled);
        }

        let pixels = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RustTable Color Zones pixels"),
            size: validated.pixel_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: true,
        });
        write_mapped_pixels(&pixels, request.pixels)?;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RustTable Color Zones readback"),
            size: validated.pixel_bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let lightness_lut =
            create_mapped_storage_buffer(device, "RustTable Color Zones lightness LUT", LUT_BYTES);
        write_mapped_floats(&lightness_lut, request.lightness_lut)?;
        let chroma_lut =
            create_mapped_storage_buffer(device, "RustTable Color Zones chroma LUT", LUT_BYTES);
        write_mapped_floats(&chroma_lut, request.chroma_lut)?;
        let hue_lut =
            create_mapped_storage_buffer(device, "RustTable Color Zones hue LUT", LUT_BYTES);
        write_mapped_floats(&hue_lut, request.hue_lut)?;
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RustTable Color Zones parameters"),
            size: PARAMS_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        write_mapped_bytes(&params, &pack_params(request, validated.pixel_count))?;

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RustTable Color Zones bindings"),
            entries: &[
                buffer_binding(0, false, validated.pixel_bytes),
                uniform_binding(1),
                buffer_binding(2, true, LUT_BYTES),
                buffer_binding(3, true, LUT_BYTES),
                buffer_binding(4, true, LUT_BYTES),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("RustTable Color Zones pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("RustTable Color Zones shader"),
            source: wgpu::ShaderSource::Wgsl(COLORZONES_SHADER_SOURCE.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("colorzones"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("colorzones"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("RustTable Color Zones bind group"),
            layout: &layout,
            entries: &[
                binding_entry(0, &pixels),
                binding_entry(1, &params),
                binding_entry(2, &lightness_lut),
                binding_entry(3, &chroma_lut),
                binding_entry(4, &hue_lut),
            ],
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("RustTable Color Zones encoder"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("colorzones"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(validated.workgroups, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&pixels, 0, &readback, 0, validated.pixel_bytes);
        if request.is_cancelled() {
            return Err(ColorZonesError::Cancelled);
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
            .map_err(|error| ColorZonesError::Poll(error.to_string()))?;
        receiver
            .recv()
            .map_err(|error| ColorZonesError::Readback(error.to_string()))?
            .map_err(ColorZonesError::Readback)?;
        let view = slice
            .get_mapped_range()
            .map_err(|error| ColorZonesError::Readback(error.to_string()))?;
        let (chunks, remainder) = view.as_chunks::<PIXEL_BYTES>();
        if !remainder.is_empty() {
            return Err(ColorZonesError::Readback(
                "mapped pixel buffer has a partial RGBA value".to_owned(),
            ));
        }
        let pixels = chunks.iter().map(pixel_from_bytes).collect::<Vec<_>>();
        drop(view);
        readback.unmap();
        if request.is_cancelled() {
            return Err(ColorZonesError::Cancelled);
        }

        debug_assert_eq!(
            Some(validated.aggregate_transient_bytes),
            aggregate_transient_bytes(validated.pixel_bytes)
        );
        Ok(ColorZonesResult {
            pixels,
            identity: request.identity,
            dispatches: 1,
        })
    }
}

fn aggregate_transient_bytes(pixel_bytes: u64) -> Option<u64> {
    pixel_bytes
        .checked_mul(3)
        .and_then(|pixels| {
            LUT_BYTES
                .checked_mul(3)
                .and_then(|luts| pixels.checked_add(luts))
        })
        .and_then(|resources| resources.checked_add(PARAMS_SIZE))
}

fn create_mapped_storage_buffer(
    device: &wgpu::Device,
    label: &'static str,
    size: u64,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: true,
    })
}

const fn buffer_binding(binding: u32, read_only: bool, size: u64) -> wgpu::BindGroupLayoutEntry {
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

const fn uniform_binding(binding: u32) -> wgpu::BindGroupLayoutEntry {
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

fn binding_entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

fn write_mapped_pixels(buffer: &wgpu::Buffer, pixels: &[[f32; 4]]) -> Result<(), ColorZonesError> {
    let slice = buffer.slice(..);
    let mut view = slice
        .get_mapped_range_mut()
        .map_err(|error| ColorZonesError::Upload(error.to_string()))?;
    let expected = pixels
        .len()
        .checked_mul(PIXEL_BYTES)
        .ok_or(ColorZonesError::SizeOverflow)?;
    if view.len() != expected {
        return Err(ColorZonesError::Upload(
            "mapped upload buffer does not match RGBA pixel packing".to_owned(),
        ));
    }
    for (pixel_index, pixel) in pixels.iter().enumerate() {
        let mut bytes = [0_u8; PIXEL_BYTES];
        let (components, remainder) = bytes.as_chunks_mut::<4>();
        debug_assert!(remainder.is_empty());
        for (component, destination) in pixel.iter().zip(components) {
            destination.copy_from_slice(&component.to_le_bytes());
        }
        let start = pixel_index
            .checked_mul(PIXEL_BYTES)
            .ok_or(ColorZonesError::SizeOverflow)?;
        let end = start
            .checked_add(PIXEL_BYTES)
            .ok_or(ColorZonesError::SizeOverflow)?;
        view.slice(start..end).copy_from_slice(&bytes);
    }
    drop(view);
    buffer.unmap();
    Ok(())
}

fn write_mapped_floats(buffer: &wgpu::Buffer, values: &[f32]) -> Result<(), ColorZonesError> {
    let slice = buffer.slice(..);
    let mut view = slice
        .get_mapped_range_mut()
        .map_err(|error| ColorZonesError::Upload(error.to_string()))?;
    let expected = values
        .len()
        .checked_mul(4)
        .ok_or(ColorZonesError::SizeOverflow)?;
    if view.len() != expected {
        return Err(ColorZonesError::Upload(
            "mapped upload buffer does not match LUT packing".to_owned(),
        ));
    }
    for (index, value) in values.iter().enumerate() {
        let start = index.checked_mul(4).ok_or(ColorZonesError::SizeOverflow)?;
        let end = start.checked_add(4).ok_or(ColorZonesError::SizeOverflow)?;
        view.slice(start..end).copy_from_slice(&value.to_le_bytes());
    }
    drop(view);
    buffer.unmap();
    Ok(())
}

fn write_mapped_bytes(buffer: &wgpu::Buffer, bytes: &[u8]) -> Result<(), ColorZonesError> {
    let slice = buffer.slice(..);
    let mut view = slice
        .get_mapped_range_mut()
        .map_err(|error| ColorZonesError::Upload(error.to_string()))?;
    if view.len() != bytes.len() {
        return Err(ColorZonesError::Upload(
            "mapped upload buffer does not match parameter packing".to_owned(),
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
    let (components, remainder) = bytes.as_chunks::<4>();
    debug_assert!(remainder.is_empty());
    std::array::from_fn(|component| f32::from_le_bytes(components[component]))
}

fn pack_params(request: ColorZonesRequest<'_>, pixel_count: u32) -> [u8; PARAMS_BYTES] {
    let words = [pixel_count, request.selection.raw(), request.mode.raw(), 0];
    let mut bytes = [0_u8; PARAMS_BYTES];
    let (chunks, remainder) = bytes.as_chunks_mut::<4>();
    debug_assert!(remainder.is_empty());
    for (chunk, word) in chunks.iter_mut().zip(words) {
        chunk.copy_from_slice(&word.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
#[path = "colorzones/tests.rs"]
mod tests;
