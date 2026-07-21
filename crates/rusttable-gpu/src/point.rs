use std::fmt;
use std::num::NonZeroU64;

use crate::{FaultState, GpuRuntime};

const WORKGROUP_SIZE: u32 = 256;
const POINT_PARAMS_SIZE: u64 = 64;
const POINT_PARAMS_BYTES: usize = 64;

/// One operation in the basic linear-light point pipeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BasicPointOperation {
    Exposure { stops: f32, black: f32 },
    LinearOffset { value: f32 },
    RgbGain { red: f32, green: f32, blue: f32 },
}

impl BasicPointOperation {
    fn entry_point(self) -> &'static str {
        match self {
            Self::Exposure { .. } => "exposure",
            Self::LinearOffset { .. } => "linear_offset",
            Self::RgbGain { .. } => "rgb_gain",
        }
    }

    fn params(self, pixel_count: u32) -> [u8; POINT_PARAMS_BYTES] {
        let mut bytes = [0_u8; POINT_PARAMS_BYTES];
        bytes[0..4].copy_from_slice(&pixel_count.to_le_bytes());
        let values = match self {
            Self::Exposure { stops, black } => [stops, 0.0, 1.0, 1.0, 1.0, 2.2, black],
            Self::LinearOffset { value } => [0.0, value, 1.0, 1.0, 1.0, 2.2, 0.0],
            Self::RgbGain { red, green, blue } => [0.0, 0.0, red, green, blue, 2.2, 0.0],
        };
        for (index, value) in values.into_iter().enumerate() {
            let offset = 28 + index * 4;
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }
}

/// A checked request for the WGPU point-operation executor.
#[derive(Debug, Clone, Copy)]
pub struct BasicPointRequest<'a> {
    /// Packed linear-light RGBA f32 pixels. Alpha is copied unchanged.
    pub pixels: &'a [f32],
    pub operations: &'a [BasicPointOperation],
}

/// Read-back from one WGPU point-operation dispatch chain.
#[derive(Debug, Clone, PartialEq)]
pub struct BasicPointResult {
    pixels: Vec<f32>,
    dispatches: u32,
}

impl BasicPointResult {
    #[must_use]
    pub fn pixels(&self) -> &[f32] {
        &self.pixels
    }

    #[must_use]
    pub const fn dispatches(&self) -> u32 {
        self.dispatches
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BasicPointError {
    CpuOnly,
    Unhealthy,
    EmptyInput,
    InvalidPixelPacking,
    TooManyPixels,
    NonFiniteInput { component: usize },
    NonFiniteParameter,
    TooManyWorkgroups,
    ShaderUnavailable,
    Poll(String),
    Readback(String),
}

impl fmt::Display for BasicPointError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CpuOnly => formatter.write_str("basic WGPU point execution is CPU-only"),
            Self::Unhealthy => formatter.write_str("basic WGPU point execution is unhealthy"),
            Self::EmptyInput => formatter.write_str("basic WGPU point input is empty"),
            Self::InvalidPixelPacking => {
                formatter.write_str("basic WGPU point input is not packed RGBA f32")
            }
            Self::TooManyPixels => formatter.write_str("basic WGPU point input exceeds u32 pixels"),
            Self::NonFiniteInput { component } => {
                write!(
                    formatter,
                    "basic WGPU point input component {component} is non-finite"
                )
            }
            Self::NonFiniteParameter => {
                formatter.write_str("basic WGPU point operation parameter is non-finite")
            }
            Self::TooManyWorkgroups => {
                formatter.write_str("basic WGPU point input exceeds the device workgroup limit")
            }
            Self::ShaderUnavailable => {
                formatter.write_str("basic WGPU point shader is unavailable")
            }
            Self::Poll(error) => write!(formatter, "basic WGPU point poll failed: {error}"),
            Self::Readback(error) => write!(formatter, "basic WGPU point readback failed: {error}"),
        }
    }
}

impl std::error::Error for BasicPointError {}

impl GpuRuntime {
    /// Executes the registered point-operation shaders in authored order.
    ///
    /// Transfer conversion remains at the typed pixelpipe boundary for this
    /// first slice. The executor therefore receives linear-light RGBA f32
    /// values and returns the same representation, preserving alpha exactly.
    /// Unsupported transfer, opacity, and non-point stages are rejected by the
    /// pixelpipe adapter before this method is called.
    #[allow(clippy::too_many_lines)]
    pub fn execute_basic_point(
        &self,
        request: BasicPointRequest<'_>,
    ) -> Result<BasicPointResult, BasicPointError> {
        validate_request(self, request)?;
        if request.operations.is_empty() {
            return Ok(BasicPointResult {
                pixels: request.pixels.to_vec(),
                dispatches: 0,
            });
        }
        let (device, queue) = self.handles().ok_or(BasicPointError::CpuOnly)?;
        let source = crate::shader::ShaderRegistry::checked_in().point_source();
        if source.is_empty() {
            return Err(BasicPointError::ShaderUnavailable);
        }
        let pixel_bytes = floats_as_bytes(request.pixels);
        let buffer_size =
            u64::try_from(pixel_bytes.len()).map_err(|_| BasicPointError::TooManyPixels)?;
        let usage = wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC;
        let input = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RustTable basic point input"),
            size: buffer_size,
            usage,
            mapped_at_creation: false,
        });
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RustTable basic point output"),
            size: buffer_size,
            usage,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RustTable basic point readback"),
            size: buffer_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&input, 0, &pixel_bytes);

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RustTable basic point bindings"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(16),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(16),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(POINT_PARAMS_SIZE),
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("RustTable basic point pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("RustTable basic point shader"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("RustTable basic point encoder"),
        });
        let mut input_buffer = &input;
        let mut output_buffer = &output;
        let mut dispatches = 0_u32;
        let workgroups =
            u32::try_from((request.pixels.len() / 4).div_ceil(WORKGROUP_SIZE as usize))
                .map_err(|_| BasicPointError::TooManyWorkgroups)?;
        let parameter_buffers = request
            .operations
            .iter()
            .map(|operation| {
                let params = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("RustTable basic point parameters"),
                    size: POINT_PARAMS_SIZE,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let params_bytes =
                    operation.params(u32::try_from(request.pixels.len() / 4).unwrap_or(u32::MAX));
                queue.write_buffer(&params, 0, &params_bytes);
                params
            })
            .collect::<Vec<_>>();
        for (operation, params) in request.operations.iter().zip(&parameter_buffers) {
            let entry_point = operation.entry_point();
            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry_point),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some(entry_point),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("RustTable basic point bind group"),
                layout: &bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: input_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: output_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: params.as_entire_binding(),
                    },
                ],
            });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some(entry_point),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &bind_group, &[]);
                pass.dispatch_workgroups(workgroups, 1, 1);
            }
            std::mem::swap(&mut input_buffer, &mut output_buffer);
            dispatches = dispatches.saturating_add(1);
        }
        encoder.copy_buffer_to_buffer(input_buffer, 0, &readback, 0, buffer_size);
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
            .map_err(|error| BasicPointError::Poll(error.to_string()))?;
        receiver
            .recv()
            .map_err(|error| BasicPointError::Readback(error.to_string()))?
            .map_err(BasicPointError::Readback)?;
        let view = slice
            .get_mapped_range()
            .map_err(|error| BasicPointError::Readback(error.to_string()))?;
        let (chunks, remainder) = view.as_chunks::<4>();
        debug_assert!(remainder.is_empty());
        let pixels = chunks
            .iter()
            .map(|bytes| f32::from_le_bytes(*bytes))
            .collect();
        drop(view);
        readback.unmap();
        Ok(BasicPointResult { pixels, dispatches })
    }
}

fn validate_request(
    runtime: &GpuRuntime,
    request: BasicPointRequest<'_>,
) -> Result<(), BasicPointError> {
    if runtime.is_cpu_only() {
        return Err(BasicPointError::CpuOnly);
    }
    if !matches!(
        runtime.snapshot().state,
        FaultState::Healthy | FaultState::Degraded
    ) {
        return Err(BasicPointError::Unhealthy);
    }
    if request.pixels.is_empty() {
        return Err(BasicPointError::EmptyInput);
    }
    if !request.pixels.len().is_multiple_of(4) {
        return Err(BasicPointError::InvalidPixelPacking);
    }
    if request.pixels.len() / 4 > u32::MAX as usize {
        return Err(BasicPointError::TooManyPixels);
    }
    if let Some(component) = request.pixels.iter().position(|value| !value.is_finite()) {
        return Err(BasicPointError::NonFiniteInput { component });
    }
    if request.operations.iter().any(|operation| match operation {
        BasicPointOperation::Exposure { stops, black } => !stops.is_finite() || !black.is_finite(),
        BasicPointOperation::LinearOffset { value } => !value.is_finite(),
        BasicPointOperation::RgbGain { red, green, blue } => {
            !red.is_finite() || !green.is_finite() || !blue.is_finite()
        }
    }) {
        return Err(BasicPointError::NonFiniteParameter);
    }
    let workgroups = request.pixels.len().div_ceil(4 * WORKGROUP_SIZE as usize) as u64;
    if workgroups > u64::from(runtime.snapshot().limits.max_workgroups_per_dimension) {
        return Err(BasicPointError::TooManyWorkgroups);
    }
    Ok(())
}

fn floats_as_bytes(values: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_parameters_follow_the_checked_in_uniform_layout() {
        let bytes = BasicPointOperation::RgbGain {
            red: 1.0,
            green: 2.0,
            blue: 3.0,
        }
        .params(7);
        assert_eq!(&bytes[0..4], &7_u32.to_le_bytes());
        assert_eq!(&bytes[36..40], &1.0_f32.to_le_bytes());
        assert_eq!(&bytes[40..44], &2.0_f32.to_le_bytes());
        assert_eq!(&bytes[44..48], &3.0_f32.to_le_bytes());
        assert_eq!(&bytes[52..56], &0.0_f32.to_le_bytes());
        assert_eq!(bytes.len(), 64);
    }

    #[tokio::test]
    async fn basic_point_dispatch_matches_linear_cpu_reference_when_gpu_is_available() {
        let Ok(runtime) = GpuRuntime::initialize(crate::GpuRuntimeConfig::default()).await else {
            return;
        };
        if runtime.is_cpu_only() {
            return;
        }
        let input = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8];
        let operations = [
            BasicPointOperation::Exposure {
                stops: 1.0,
                black: 0.0,
            },
            BasicPointOperation::LinearOffset { value: 0.1 },
            BasicPointOperation::RgbGain {
                red: 0.5,
                green: 1.5,
                blue: 2.0,
            },
        ];
        let result = runtime
            .execute_basic_point(BasicPointRequest {
                pixels: &input,
                operations: &operations,
            })
            .expect("basic point dispatch");
        let expected = [0.15, 0.75, 1.4, 0.4, 0.55, 1.95, 3.0, 0.8];
        for (actual, expected) in result.pixels().iter().zip(expected) {
            assert!(
                (actual - expected).abs() < 0.00001,
                "{actual} != {expected}"
            );
        }
        assert_eq!(result.dispatches(), 3);
    }

    #[tokio::test]
    async fn exposure_black_level_dispatch_matches_darktable_formula() {
        let Ok(runtime) = GpuRuntime::initialize(crate::GpuRuntimeConfig::default()).await else {
            return;
        };
        if runtime.is_cpu_only() {
            return;
        }
        let input = [0.5, 0.25, 0.75, 0.4];
        let operations = [BasicPointOperation::Exposure {
            stops: 1.0,
            black: 0.125,
        }];
        let result = runtime
            .execute_basic_point(BasicPointRequest {
                pixels: &input,
                operations: &operations,
            })
            .expect("black-level point dispatch");
        let expected = [1.0, 1.0 / 3.0, 5.0 / 3.0, 0.4];
        for (actual, expected) in result.pixels().iter().zip(expected) {
            assert!(
                (actual - expected).abs() < 0.00001,
                "{actual} != {expected}"
            );
        }
        assert_eq!(result.dispatches(), 1);
    }
}
