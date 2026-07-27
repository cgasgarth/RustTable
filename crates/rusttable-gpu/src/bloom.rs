//! Safe WGPU port of Darktable's `src/iop/bloom.c` `process_cl` path and
//! `data/kernels/bloom.cl`.

use std::fmt;
use std::num::NonZeroU64;

use crate::{CancellationToken, FaultState, GpuRuntime};

/// Number of retained box-mean horizontal/vertical iteration pairs.
pub const BLOOM_BOX_ITERATIONS: u32 = 8;
/// Number of temporary lightness buffers in the retained `OpenCL` bucket chain.
pub const BLOOM_TEMPORARY_BUCKETS: u32 = 4;
/// Maximum scaled radius accepted by the retained implementation.
pub const BLOOM_MAX_RADIUS: u32 = 256;

const PARAMS_SIZE: u64 = 32;
const PARAMS_BYTES: usize = 32;
const PIXEL_BYTES: u64 = 16;
const PIXEL_BYTES_USIZE: usize = 16;
const LIGHTNESS_BYTES: u64 = 4;
const DEVICE_PIXEL_BUFFER_COUNT: u64 = 3;
const HOST_PIXEL_BUFFER_COUNT: u64 = 1;
const THRESHOLD_WORKGROUP: [u32; 3] = [16, 16, 1];
const HORIZONTAL_WORKGROUP: [u32; 3] = [256, 1, 1];
const VERTICAL_WORKGROUP: [u32; 3] = [1, 256, 1];
const BLOOM_SHADER_SOURCE: &str = include_str!("../shaders/bloom.wgsl");

/// A checked full-frame or overlap-expanded tile request for Bloom.
///
/// `radius` is resolved by the canonical Bloom geometry owner before this GPU
/// boundary. It therefore already includes `roi_in.scale / piece.iscale`, as
/// retained `process_cl` does, and is capped at [`BLOOM_MAX_RADIUS`].
#[derive(Debug, Clone, Copy)]
pub struct BloomRequest<'a> {
    pixels: &'a [[f32; 4]],
    width: usize,
    height: usize,
    radius: u32,
    threshold: f32,
    strength: f32,
    transient_memory_budget_bytes: u64,
    cancellation: Option<&'a CancellationToken>,
}

impl<'a> BloomRequest<'a> {
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        pixels: &'a [[f32; 4]],
        width: usize,
        height: usize,
        radius: u32,
        threshold: f32,
        strength: f32,
        transient_memory_budget_bytes: u64,
    ) -> Self {
        Self {
            pixels,
            width,
            height,
            radius,
            threshold,
            strength,
            transient_memory_budget_bytes,
            cancellation: None,
        }
    }

    /// Attaches cancellation checks before qualification, before submission,
    /// and after readback.
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

/// Read-back Lab pixels and the number of compute dispatches submitted.
#[derive(Debug, Clone, PartialEq)]
pub struct BloomResult {
    pixels: Vec<[f32; 4]>,
    dispatches: u32,
}

impl BloomResult {
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

/// Checked qualification or execution failure from the concrete Bloom path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BloomError {
    CpuOnly,
    Unhealthy,
    BufferShape { expected: usize, actual: usize },
    InvalidDimensions,
    InvalidRadius { radius: u32 },
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

impl fmt::Display for BloomError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CpuOnly => formatter.write_str("Bloom WGPU execution is CPU-only"),
            Self::Unhealthy => formatter.write_str("Bloom WGPU execution is unhealthy"),
            Self::BufferShape { expected, actual } => {
                write!(
                    formatter,
                    "Bloom WGPU expected {expected} pixels, got {actual}"
                )
            }
            Self::InvalidDimensions => formatter.write_str("Bloom WGPU dimensions are invalid"),
            Self::InvalidRadius { radius } => write!(
                formatter,
                "Bloom WGPU radius {radius} exceeds the retained maximum"
            ),
            Self::InvalidParameter(name) => {
                write!(formatter, "Bloom WGPU {name} parameter is invalid")
            }
            Self::NonFiniteInput { pixel, component } => write!(
                formatter,
                "Bloom WGPU input has a non-finite component {component} at pixel {pixel}"
            ),
            Self::NonFiniteOutput { pixel, component } => write!(
                formatter,
                "Bloom WGPU produced a non-finite component {component} at pixel {pixel}"
            ),
            Self::Cancelled => formatter.write_str("Bloom WGPU execution was cancelled"),
            Self::SizeOverflow => formatter.write_str("Bloom WGPU size overflowed"),
            Self::InvalidMemoryBudget => {
                formatter.write_str("Bloom WGPU transient-memory budget must be nonzero")
            }
            Self::BufferLimit { required, limit } => write!(
                formatter,
                "Bloom WGPU buffer requires {required} bytes; device limit is {limit}"
            ),
            Self::AggregateMemoryLimit { required, limit } => write!(
                formatter,
                "Bloom WGPU transient allocations require {required} bytes; transient-memory budget is {limit}"
            ),
            Self::InsufficientComputeLimits => {
                formatter.write_str("Bloom WGPU device compute or binding limits are insufficient")
            }
            Self::TooManyWorkgroups => {
                formatter.write_str("Bloom WGPU dispatch exceeds the device workgroup limit")
            }
            Self::ShaderUnavailable => formatter.write_str("Bloom WGPU shader is unavailable"),
            Self::Upload(error) => write!(formatter, "Bloom WGPU upload failed: {error}"),
            Self::Poll(error) => write!(formatter, "Bloom WGPU poll failed: {error}"),
            Self::Readback(error) => write!(formatter, "Bloom WGPU readback failed: {error}"),
        }
    }
}

impl std::error::Error for BloomError {}

#[derive(Debug, Clone, Copy)]
struct DispatchPlan {
    threshold_mix: [u32; 3],
    horizontal: [u32; 3],
    vertical: [u32; 3],
}

#[derive(Debug)]
struct ValidatedRequest {
    width: u32,
    height: u32,
    radius: u32,
    scale: f32,
    threshold: f32,
    pixel_bytes: u64,
    lightness_bytes: u64,
    aggregate_transient_bytes: u64,
    dispatch: DispatchPlan,
}

impl ValidatedRequest {
    fn new(request: BloomRequest<'_>, limits: &wgpu::Limits) -> Result<Self, BloomError> {
        let expected = request
            .width
            .checked_mul(request.height)
            .ok_or(BloomError::SizeOverflow)?;
        if request.width == 0
            || request.height == 0
            || request.width > i32::MAX as usize
            || request.height > i32::MAX as usize
        {
            return Err(BloomError::InvalidDimensions);
        }
        if request.pixels.len() != expected {
            return Err(BloomError::BufferShape {
                expected,
                actual: request.pixels.len(),
            });
        }
        if request.radius > BLOOM_MAX_RADIUS {
            return Err(BloomError::InvalidRadius {
                radius: request.radius,
            });
        }
        if !request.threshold.is_finite() || !(0.0..=100.0).contains(&request.threshold) {
            return Err(BloomError::InvalidParameter("threshold"));
        }
        if !request.strength.is_finite() || !(0.0..=100.0).contains(&request.strength) {
            return Err(BloomError::InvalidParameter("strength"));
        }
        if let Some((pixel, component)) = first_non_finite(request.pixels) {
            return Err(BloomError::NonFiniteInput { pixel, component });
        }
        if request.transient_memory_budget_bytes == 0 {
            return Err(BloomError::InvalidMemoryBudget);
        }

        let width = u32::try_from(request.width).map_err(|_| BloomError::SizeOverflow)?;
        let height = u32::try_from(request.height).map_err(|_| BloomError::SizeOverflow)?;
        let pixel_count = u64::try_from(expected).map_err(|_| BloomError::SizeOverflow)?;
        let pixel_bytes = pixel_count
            .checked_mul(PIXEL_BYTES)
            .ok_or(BloomError::SizeOverflow)?;
        let lightness_bytes = pixel_count
            .checked_mul(LIGHTNESS_BYTES)
            .ok_or(BloomError::SizeOverflow)?;
        let aggregate_transient_bytes = transient_bytes(pixel_bytes, lightness_bytes)?;
        if aggregate_transient_bytes > request.transient_memory_budget_bytes {
            return Err(BloomError::AggregateMemoryLimit {
                required: aggregate_transient_bytes,
                limit: request.transient_memory_budget_bytes,
            });
        }

        let storage_limit = limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size);
        for required in [pixel_bytes, lightness_bytes] {
            if required > storage_limit {
                return Err(BloomError::BufferLimit {
                    required,
                    limit: storage_limit,
                });
            }
        }
        let uniform_limit = limits
            .max_uniform_buffer_binding_size
            .min(limits.max_buffer_size);
        if PARAMS_SIZE > uniform_limit {
            return Err(BloomError::BufferLimit {
                required: PARAMS_SIZE,
                limit: uniform_limit,
            });
        }
        if limits.max_compute_invocations_per_workgroup < 256
            || limits.max_compute_workgroup_size_x < HORIZONTAL_WORKGROUP[0]
            || limits.max_compute_workgroup_size_y < VERTICAL_WORKGROUP[1]
            || limits.max_compute_workgroup_size_z < 1
            || limits.max_bind_groups < 1
            || limits.max_bindings_per_bind_group < 5
            || limits.max_storage_buffers_per_shader_stage < 3
            || limits.max_uniform_buffers_per_shader_stage < 1
        {
            return Err(BloomError::InsufficientComputeLimits);
        }

        let dispatch = DispatchPlan {
            threshold_mix: [
                width.div_ceil(THRESHOLD_WORKGROUP[0]),
                height.div_ceil(THRESHOLD_WORKGROUP[1]),
                1,
            ],
            horizontal: [width.div_ceil(HORIZONTAL_WORKGROUP[0]), height, 1],
            vertical: [width, height.div_ceil(VERTICAL_WORKGROUP[1]), 1],
        };
        if [
            dispatch.threshold_mix,
            dispatch.horizontal,
            dispatch.vertical,
        ]
        .into_iter()
        .flatten()
        .any(|groups| groups > limits.max_compute_workgroups_per_dimension)
        {
            return Err(BloomError::TooManyWorkgroups);
        }

        let strength_exponent = (request.strength + 1.0).min(100.0) / 100.0;
        let scale = 1.0 / (-strength_exponent).exp2();
        if !scale.is_finite() {
            return Err(BloomError::InvalidParameter("strength"));
        }
        Ok(Self {
            width,
            height,
            radius: request.radius,
            scale,
            threshold: request.threshold,
            pixel_bytes,
            lightness_bytes,
            aggregate_transient_bytes,
            dispatch,
        })
    }
}

/// Returns the exact explicit transient allocation footprint of this boundary.
///
/// This includes input, output, readback, four retained lightness buckets, the
/// uniform block, and the host result vector. Uploads are written directly into
/// mapped buffers and therefore require no serialized staging vectors.
pub fn bloom_transient_memory_bytes(width: usize, height: usize) -> Result<u64, BloomError> {
    if width == 0 || height == 0 {
        return Err(BloomError::InvalidDimensions);
    }
    let pixels = width.checked_mul(height).ok_or(BloomError::SizeOverflow)?;
    let pixels = u64::try_from(pixels).map_err(|_| BloomError::SizeOverflow)?;
    let pixel_bytes = pixels
        .checked_mul(PIXEL_BYTES)
        .ok_or(BloomError::SizeOverflow)?;
    let lightness_bytes = pixels
        .checked_mul(LIGHTNESS_BYTES)
        .ok_or(BloomError::SizeOverflow)?;
    transient_bytes(pixel_bytes, lightness_bytes)
}

fn transient_bytes(pixel_bytes: u64, lightness_bytes: u64) -> Result<u64, BloomError> {
    pixel_bytes
        .checked_mul(DEVICE_PIXEL_BUFFER_COUNT + HOST_PIXEL_BUFFER_COUNT)
        .and_then(|bytes| {
            lightness_bytes
                .checked_mul(u64::from(BLOOM_TEMPORARY_BUCKETS))
                .and_then(|temporary| bytes.checked_add(temporary))
        })
        .and_then(|bytes| bytes.checked_add(PARAMS_SIZE))
        .ok_or(BloomError::SizeOverflow)
}

impl GpuRuntime {
    /// Runs threshold, eight horizontal/vertical box pairs, and recombination.
    #[allow(clippy::too_many_lines)]
    pub fn execute_bloom(&self, request: BloomRequest<'_>) -> Result<BloomResult, BloomError> {
        if request.is_cancelled() {
            return Err(BloomError::Cancelled);
        }
        if self.is_cpu_only() {
            return Err(BloomError::CpuOnly);
        }
        if !matches!(
            self.snapshot().state,
            FaultState::Healthy | FaultState::Degraded
        ) {
            return Err(BloomError::Unhealthy);
        }
        let (device, queue) = self.handles().ok_or(BloomError::CpuOnly)?;
        if BLOOM_SHADER_SOURCE.is_empty() {
            return Err(BloomError::ShaderUnavailable);
        }
        let validated = ValidatedRequest::new(request, &device.limits())?;

        let input = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RustTable Bloom input"),
            size: validated.pixel_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        write_mapped_pixels(&input, request.pixels)?;
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RustTable Bloom output"),
            size: validated.pixel_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RustTable Bloom readback"),
            size: validated.pixel_bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let temporary = (0..BLOOM_TEMPORARY_BUCKETS)
            .map(|index| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(match index {
                        0 => "RustTable Bloom lightness bucket 0",
                        1 => "RustTable Bloom lightness bucket 1",
                        2 => "RustTable Bloom lightness bucket 2",
                        _ => "RustTable Bloom lightness bucket 3",
                    }),
                    size: validated.lightness_bytes,
                    usage: wgpu::BufferUsages::STORAGE,
                    mapped_at_creation: false,
                })
            })
            .collect::<Vec<_>>();
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RustTable Bloom parameters"),
            size: PARAMS_SIZE,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: true,
        });
        write_mapped_bytes(&params, &pack_params(&validated))?;

        let threshold_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RustTable Bloom threshold bindings"),
            entries: &[
                storage_binding(0, true, validated.pixel_bytes),
                storage_binding(2, false, validated.lightness_bytes),
                uniform_binding(4),
            ],
        });
        let blur_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RustTable Bloom blur bindings"),
            entries: &[
                storage_binding(1, true, validated.lightness_bytes),
                storage_binding(2, false, validated.lightness_bytes),
                uniform_binding(4),
            ],
        });
        let mix_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RustTable Bloom mix bindings"),
            entries: &[
                storage_binding(0, true, validated.pixel_bytes),
                storage_binding(1, true, validated.lightness_bytes),
                storage_binding(3, false, validated.pixel_bytes),
                uniform_binding(4),
            ],
        });
        let threshold_pipeline_layout =
            pipeline_layout(device, "Bloom threshold", &threshold_layout);
        let blur_pipeline_layout = pipeline_layout(device, "Bloom blur", &blur_layout);
        let mix_pipeline_layout = pipeline_layout(device, "Bloom mix", &mix_layout);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("RustTable Bloom shader"),
            source: wgpu::ShaderSource::Wgsl(BLOOM_SHADER_SOURCE.into()),
        });
        let threshold_pipeline = compute_pipeline(
            device,
            &threshold_pipeline_layout,
            &shader,
            "bloom_threshold",
        );
        let horizontal_pipeline =
            compute_pipeline(device, &blur_pipeline_layout, &shader, "bloom_hblur");
        let vertical_pipeline =
            compute_pipeline(device, &blur_pipeline_layout, &shader, "bloom_vblur");
        let mix_pipeline = compute_pipeline(device, &mix_pipeline_layout, &shader, "bloom_mix");

        let mut state = 0_usize;
        let threshold_output = bucket_next(&mut state);
        let threshold_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("RustTable Bloom threshold group"),
            layout: &threshold_layout,
            entries: &[
                buffer_entry(0, &input),
                buffer_entry(2, &temporary[threshold_output]),
                buffer_entry(4, &params),
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("RustTable Bloom encoder"),
        });
        dispatch(
            &mut encoder,
            "Bloom threshold",
            &threshold_pipeline,
            &threshold_group,
            validated.dispatch.threshold_mix,
        );
        let mut current = threshold_output;
        let mut dispatches = 1_u32;
        if validated.radius != 0 {
            for _ in 0..BLOOM_BOX_ITERATIONS {
                let horizontal_output = bucket_next(&mut state);
                let horizontal_group = blur_group(
                    device,
                    &blur_layout,
                    "RustTable Bloom horizontal group",
                    &temporary[current],
                    &temporary[horizontal_output],
                    &params,
                );
                dispatch(
                    &mut encoder,
                    "Bloom horizontal blur",
                    &horizontal_pipeline,
                    &horizontal_group,
                    validated.dispatch.horizontal,
                );

                let vertical_output = bucket_next(&mut state);
                let vertical_group = blur_group(
                    device,
                    &blur_layout,
                    "RustTable Bloom vertical group",
                    &temporary[horizontal_output],
                    &temporary[vertical_output],
                    &params,
                );
                dispatch(
                    &mut encoder,
                    "Bloom vertical blur",
                    &vertical_pipeline,
                    &vertical_group,
                    validated.dispatch.vertical,
                );
                current = vertical_output;
                dispatches = dispatches.checked_add(2).ok_or(BloomError::SizeOverflow)?;
            }
        }
        let mix_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("RustTable Bloom mix group"),
            layout: &mix_layout,
            entries: &[
                buffer_entry(0, &input),
                buffer_entry(1, &temporary[current]),
                buffer_entry(3, &output),
                buffer_entry(4, &params),
            ],
        });
        dispatch(
            &mut encoder,
            "Bloom mix",
            &mix_pipeline,
            &mix_group,
            validated.dispatch.threshold_mix,
        );
        dispatches = dispatches.checked_add(1).ok_or(BloomError::SizeOverflow)?;
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, validated.pixel_bytes);
        if request.is_cancelled() {
            return Err(BloomError::Cancelled);
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
            .map_err(|error| BloomError::Poll(error.to_string()))?;
        receiver
            .recv()
            .map_err(|error| BloomError::Readback(error.to_string()))?
            .map_err(BloomError::Readback)?;
        let view = slice
            .get_mapped_range()
            .map_err(|error| BloomError::Readback(error.to_string()))?;
        let (chunks, remainder) = view.as_chunks::<16>();
        if !remainder.is_empty() {
            return Err(BloomError::Readback(
                "mapped pixel buffer has a partial RGBA value".to_owned(),
            ));
        }
        let pixels = chunks.iter().map(pixel_from_bytes).collect::<Vec<_>>();
        drop(view);
        readback.unmap();
        if request.is_cancelled() {
            return Err(BloomError::Cancelled);
        }
        if let Some((pixel, component)) = first_non_finite(&pixels) {
            return Err(BloomError::NonFiniteOutput { pixel, component });
        }
        debug_assert_eq!(
            validated.aggregate_transient_bytes,
            transient_bytes(validated.pixel_bytes, validated.lightness_bytes)?
        );
        Ok(BloomResult { pixels, dispatches })
    }
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

fn buffer_entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

fn blur_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    label: &str,
    input: &wgpu::Buffer,
    output: &wgpu::Buffer,
    params: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            buffer_entry(1, input),
            buffer_entry(2, output),
            buffer_entry(4, params),
        ],
    })
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

fn bucket_next(state: &mut usize) -> usize {
    *state = if *state + 1 == BLOOM_TEMPORARY_BUCKETS as usize {
        0
    } else {
        *state + 1
    };
    *state
}

fn pack_params(validated: &ValidatedRequest) -> [u8; PARAMS_BYTES] {
    let words = [
        validated.width,
        validated.height,
        validated.radius,
        0,
        validated.scale.to_bits(),
        validated.threshold.to_bits(),
        0,
        0,
    ];
    let mut bytes = [0_u8; PARAMS_BYTES];
    let (chunks, remainder) = bytes.as_chunks_mut::<4>();
    debug_assert!(remainder.is_empty());
    for (chunk, word) in chunks.iter_mut().zip(words) {
        chunk.copy_from_slice(&word.to_le_bytes());
    }
    bytes
}

fn write_mapped_pixels(buffer: &wgpu::Buffer, pixels: &[[f32; 4]]) -> Result<(), BloomError> {
    let slice = buffer.slice(..);
    let mut view = slice
        .get_mapped_range_mut()
        .map_err(|error| BloomError::Upload(error.to_string()))?;
    let expected = pixels
        .len()
        .checked_mul(PIXEL_BYTES_USIZE)
        .ok_or(BloomError::SizeOverflow)?;
    if view.len() != expected {
        return Err(BloomError::Upload(
            "mapped upload buffer does not match RGBA pixel packing".to_owned(),
        ));
    }
    for (pixel_index, pixel) in pixels.iter().enumerate() {
        let start = pixel_index
            .checked_mul(PIXEL_BYTES_USIZE)
            .ok_or(BloomError::SizeOverflow)?;
        for (component_index, component) in pixel.iter().enumerate() {
            let component_start = start
                .checked_add(component_index * 4)
                .ok_or(BloomError::SizeOverflow)?;
            view.slice(component_start..component_start + 4)
                .copy_from_slice(&component.to_le_bytes());
        }
    }
    drop(view);
    buffer.unmap();
    Ok(())
}

fn write_mapped_bytes(buffer: &wgpu::Buffer, bytes: &[u8]) -> Result<(), BloomError> {
    let slice = buffer.slice(..);
    let mut view = slice
        .get_mapped_range_mut()
        .map_err(|error| BloomError::Upload(error.to_string()))?;
    if view.len() != bytes.len() {
        return Err(BloomError::Upload(
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
                .expect("RGBA component has four bytes"),
        )
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::cast_possible_truncation)]

    use super::*;

    const TEST_BUDGET: u64 = 512 * 1024 * 1024;

    fn request(pixels: &[[f32; 4]], width: usize, height: usize) -> BloomRequest<'_> {
        BloomRequest::new(pixels, width, height, 1, 0.0, 0.0, TEST_BUDGET)
    }

    fn assert_close(actual: f32, expected: f32) {
        let tolerance = 0.003 * expected.abs().max(1.0);
        assert!(
            (actual - expected).abs() <= tolerance,
            "expected {expected}, got {actual} (tolerance {tolerance})"
        );
    }

    #[test]
    fn shader_preserves_the_four_native_entry_points_and_workgroups() {
        let module = naga::front::wgsl::parse_str(BLOOM_SHADER_SOURCE).expect("Bloom WGSL syntax");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("Bloom WGSL validation");
        let entries = module
            .entry_points
            .iter()
            .map(|entry| (entry.name.as_str(), entry.workgroup_size))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(entries.len(), 4);
        assert_eq!(entries["bloom_threshold"], THRESHOLD_WORKGROUP);
        assert_eq!(entries["bloom_hblur"], HORIZONTAL_WORKGROUP);
        assert_eq!(entries["bloom_vblur"], VERTICAL_WORKGROUP);
        assert_eq!(entries["bloom_mix"], THRESHOLD_WORKGROUP);
    }

    #[test]
    fn bucket_chain_runs_exactly_eight_horizontal_vertical_pairs() {
        let mut state = 0;
        assert_eq!(bucket_next(&mut state), 1); // threshold
        let mut pairs = Vec::new();
        for _ in 0..BLOOM_BOX_ITERATIONS {
            pairs.push((bucket_next(&mut state), bucket_next(&mut state)));
        }
        assert_eq!(pairs.len(), 8);
        assert_eq!(pairs[0], (2, 3));
        assert_eq!(pairs[1], (0, 1));
        assert_eq!(pairs[7], (0, 1));
        assert_eq!(state, 1);
    }

    #[test]
    fn exact_allocation_budget_counts_every_owned_resource() {
        let pixels = [[50.0, 0.0, 0.0, 1.0]];
        let required = bloom_transient_memory_bytes(1, 1).expect("footprint");
        assert_eq!(
            required,
            4 * PIXEL_BYTES + 4 * LIGHTNESS_BYTES + PARAMS_SIZE
        );
        let limits = wgpu::Limits::downlevel_defaults();
        assert!(matches!(
            ValidatedRequest::new(
                BloomRequest::new(&pixels, 1, 1, 1, 0.0, 0.0, required - 1),
                &limits
            ),
            Err(BloomError::AggregateMemoryLimit {
                required: actual,
                limit
            }) if actual == required && limit == required - 1
        ));
        let validated = ValidatedRequest::new(
            BloomRequest::new(&pixels, 1, 1, 1, 0.0, 0.0, required),
            &limits,
        )
        .expect("exact budget");
        assert_eq!(validated.aggregate_transient_bytes, required);
    }

    #[test]
    fn qualification_rejects_native_boundary_violations() {
        let pixels = [[50.0, 0.0, 0.0, 1.0]];
        let limits = wgpu::Limits::downlevel_defaults();
        assert!(matches!(
            ValidatedRequest::new(request(&pixels, 2, 1), &limits),
            Err(BloomError::BufferShape { .. })
        ));
        assert!(matches!(
            ValidatedRequest::new(
                BloomRequest::new(&pixels, 1, 1, BLOOM_MAX_RADIUS + 1, 0.0, 0.0, TEST_BUDGET),
                &limits
            ),
            Err(BloomError::InvalidRadius { .. })
        ));
        assert!(matches!(
            ValidatedRequest::new(
                BloomRequest::new(&pixels, 1, 1, 1, f32::NAN, 0.0, TEST_BUDGET),
                &limits
            ),
            Err(BloomError::InvalidParameter("threshold"))
        ));
        let invalid = [[50.0, 0.0, f32::INFINITY, 1.0]];
        assert!(matches!(
            ValidatedRequest::new(request(&invalid, 1, 1), &limits),
            Err(BloomError::NonFiniteInput {
                pixel: 0,
                component: 2
            })
        ));
    }

    #[test]
    fn strength_scale_and_parameter_layout_match_process_cl() {
        let pixels = [[50.0, 0.0, 0.0, 1.0]];
        let validated = ValidatedRequest::new(
            BloomRequest::new(&pixels, 1, 1, 7, 90.0, 25.0, TEST_BUDGET),
            &wgpu::Limits::downlevel_defaults(),
        )
        .expect("request");
        let expected = 1.0 / (-0.26_f32).exp2();
        assert_eq!(validated.scale.to_bits(), expected.to_bits());
        let packed = pack_params(&validated);
        assert_eq!(u32::from_le_bytes(packed[8..12].try_into().unwrap()), 7);
        assert_eq!(
            f32::from_le_bytes(packed[16..20].try_into().unwrap()).to_bits(),
            expected.to_bits()
        );
        assert_eq!(
            f32::from_le_bytes(packed[20..24].try_into().unwrap()).to_bits(),
            90.0_f32.to_bits()
        );
    }

    #[test]
    fn cancellation_is_a_qualification_seam_without_geometry_changes() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let pixels = [[50.0, 0.0, 0.0, 1.0]];
        let request = request(&pixels, 1, 1).with_cancellation(&cancellation);
        assert!(request.is_cancelled());
        ValidatedRequest::new(request, &wgpu::Limits::downlevel_defaults())
            .expect("geometry remains qualified");
    }

    #[tokio::test]
    async fn backend_clamp_edge_vector_matches_opencl_semantics_when_available() {
        let Ok(runtime) = GpuRuntime::initialize(crate::GpuRuntimeConfig::default()).await else {
            return;
        };
        if runtime.is_cpu_only() {
            return;
        }
        let input = [
            [50.0, -10.0, 20.0, 0.1],
            [0.0, -9.0, 19.0, 0.2],
            [0.0, -8.0, 18.0, 0.3],
            [0.0, -7.0, 17.0, 0.4],
            [0.0, -6.0, 16.0, 0.5],
            [0.0, -5.0, 15.0, 0.6],
        ];
        let result = runtime
            .execute_bloom(request(&input, 3, 2))
            .expect("Bloom GPU dispatch");
        assert_eq!(result.dispatches(), 18);

        // Backend-specific edge probes derived from OpenCL's clamp-edge sampler.
        // Tolerance permits backend exp2/accumulation differences; this does not
        // impose whole-frame bit identity on CPU and GPU border algorithms.
        for (index, expected) in [
            (0, 54.441_883_f32),
            (2, 7.901_39_f32),
            (3, 8.881_065_f32),
            (5, 7.898_971_f32),
        ] {
            assert_close(result.pixels()[index][0], expected);
            assert_eq!(result.pixels()[index][1..], input[index][1..]);
        }
    }

    #[tokio::test]
    async fn zero_radius_submits_only_threshold_and_mix_when_available() {
        let Ok(runtime) = GpuRuntime::initialize(crate::GpuRuntimeConfig::default()).await else {
            return;
        };
        if runtime.is_cpu_only() {
            return;
        }
        let input = [[10.0, 2.0, 3.0, 0.5], [90.0, 4.0, 5.0, 0.75]];
        let result = runtime
            .execute_bloom(BloomRequest::new(&input, 2, 1, 0, 50.0, 0.0, TEST_BUDGET))
            .expect("zero-radius Bloom");
        assert_eq!(result.dispatches(), 2);
        assert_close(result.pixels()[0][0], input[0][0]);
        assert_eq!(result.pixels()[0][1..], input[0][1..]);
        assert_close(
            result.pixels()[1][0],
            100.0 - (100.0 - 90.0) * (100.0 - 90.0 * 1.006_955_6) / 100.0,
        );
        assert_eq!(result.pixels()[1][1..], input[1][1..]);
    }
}
