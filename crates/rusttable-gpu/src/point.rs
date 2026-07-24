use std::fmt;
use std::num::NonZeroU64;

use crate::shader::{BindingReflection, BindingResourceKind, ShaderRegistry};
use crate::{FaultState, GpuRuntime};

const WORKGROUP_SIZE: u32 = 256;
const POINT_PARAMS_SIZE: u64 = 64;
const POINT_PARAMS_BYTES: usize = 64;
const BASICADJ_PARAMS_SIZE: u64 = 48;
const BASICADJ_PARAMS_BYTES: usize = 48;

#[derive(Debug)]
struct PointEntryContract<'a> {
    source: &'a str,
    layout_entries: Vec<wgpu::BindGroupLayoutEntry>,
    params_size: u64,
    basic_params_size: Option<u64>,
}

struct PointParameterBuffers {
    params: wgpu::Buffer,
    basic_params: Option<wgpu::Buffer>,
}

/// Frozen scalar coefficients for one atomic basicadj stage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasicAdjPointParameters {
    pub black_point: f32,
    pub scale: f32,
    pub gamma: f32,
    pub middle_grey: f32,
    pub contrast: f32,
    pub hlcomp: f32,
    pub hlrange: f32,
    pub preserve_colors: i32,
    pub saturation: f32,
    pub vibrance: f32,
}

/// One operation in the basic linear-light point pipeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BasicPointOperation {
    BasicAdj(BasicAdjPointParameters),
    Exposure { stops: f32, black: f32 },
    LinearOffset { value: f32 },
    RgbGain { red: f32, green: f32, blue: f32 },
}

impl BasicPointOperation {
    fn entry_point(self) -> &'static str {
        match self {
            Self::BasicAdj(_) => "basicadj",
            Self::Exposure { .. } => "exposure",
            Self::LinearOffset { .. } => "linear_offset",
            Self::RgbGain { .. } => "rgb_gain",
        }
    }

    fn params(self, pixel_count: u32) -> [u8; POINT_PARAMS_BYTES] {
        let mut bytes = [0_u8; POINT_PARAMS_BYTES];
        bytes[0..4].copy_from_slice(&pixel_count.to_le_bytes());
        let values = match self {
            Self::BasicAdj(_) => [0.0, 0.0, 1.0, 1.0, 1.0, 2.2, 0.0],
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

    fn basic_params(self) -> [u8; BASICADJ_PARAMS_BYTES] {
        let mut bytes = [0_u8; BASICADJ_PARAMS_BYTES];
        let Self::BasicAdj(parameters) = self else {
            return bytes;
        };
        for (index, value) in [
            parameters.black_point,
            parameters.scale,
            parameters.gamma,
            parameters.middle_grey,
            parameters.contrast,
            parameters.hlcomp,
            parameters.hlrange,
            parameters.saturation,
            parameters.vibrance,
        ]
        .into_iter()
        .enumerate()
        {
            let offset = index * 4;
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[36..40].copy_from_slice(&parameters.preserve_colors.to_le_bytes());
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
        let registry = ShaderRegistry::checked_in();
        let contracts = request
            .operations
            .iter()
            .map(|operation| point_entry_contract(registry, operation.entry_point()))
            .collect::<Result<Vec<_>, _>>()?;
        let source = contracts.first().map_or("", |contract| contract.source);
        if source.is_empty() || contracts.iter().any(|contract| contract.source != source) {
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
            .zip(&contracts)
            .map(|(operation, contract)| {
                let params = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("RustTable basic point parameters"),
                    size: contract.params_size,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let params_bytes =
                    operation.params(u32::try_from(request.pixels.len() / 4).unwrap_or(u32::MAX));
                queue.write_buffer(&params, 0, &params_bytes);
                let basic_params = contract.basic_params_size.map(|size| {
                    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("RustTable basicadj parameters"),
                        size,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                    queue.write_buffer(&buffer, 0, &operation.basic_params());
                    buffer
                });
                PointParameterBuffers {
                    params,
                    basic_params,
                }
            })
            .collect::<Vec<_>>();
        for ((operation, contract), parameter_buffers) in request
            .operations
            .iter()
            .zip(&contracts)
            .zip(&parameter_buffers)
        {
            let entry_point = operation.entry_point();
            let bind_group_layout =
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some(entry_point),
                    entries: &contract.layout_entries,
                });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(entry_point),
                bind_group_layouts: &[Some(&bind_group_layout)],
                immediate_size: 0,
            });
            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry_point),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some(entry_point),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            });
            let mut bind_group_entries = vec![
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
                    resource: parameter_buffers.params.as_entire_binding(),
                },
            ];
            if let Some(basic_params) = &parameter_buffers.basic_params {
                bind_group_entries.push(wgpu::BindGroupEntry {
                    binding: 3,
                    resource: basic_params.as_entire_binding(),
                });
            }
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(entry_point),
                layout: &bind_group_layout,
                entries: &bind_group_entries,
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

fn point_entry_contract<'a>(
    registry: &'a ShaderRegistry,
    entry_point: &str,
) -> Result<PointEntryContract<'a>, BasicPointError> {
    let entry = registry
        .find("rusttable.point", entry_point)
        .ok_or(BasicPointError::ShaderUnavailable)?;
    let reflection = &entry.reflection;
    let expected_bindings: &[u32] = if entry_point == "basicadj" {
        &[0, 1, 2, 3]
    } else {
        &[0, 1, 2]
    };
    if entry.expanded_source.is_empty()
        || reflection.entry_point != entry_point
        || !reflection.stage.eq_ignore_ascii_case("compute")
        || reflection.workgroup_size != [WORKGROUP_SIZE, 1, 1]
        || reflection
            .bindings
            .iter()
            .map(|binding| binding.binding)
            .ne(expected_bindings.iter().copied())
    {
        return Err(BasicPointError::ShaderUnavailable);
    }
    let layout_entries = reflection
        .bindings
        .iter()
        .map(point_layout_entry)
        .collect::<Option<Vec<_>>>()
        .ok_or(BasicPointError::ShaderUnavailable)?;
    let params_size = reflected_uniform_size(&reflection.bindings, 2)
        .filter(|size| *size == POINT_PARAMS_SIZE)
        .ok_or(BasicPointError::ShaderUnavailable)?;
    let basic_params_size = reflected_uniform_size(&reflection.bindings, 3);
    if basic_params_size.is_some() != (entry_point == "basicadj")
        || basic_params_size.is_some_and(|size| size != BASICADJ_PARAMS_SIZE)
    {
        return Err(BasicPointError::ShaderUnavailable);
    }
    Ok(PointEntryContract {
        source: &entry.expanded_source,
        layout_entries,
        params_size,
        basic_params_size,
    })
}

fn point_layout_entry(binding: &BindingReflection) -> Option<wgpu::BindGroupLayoutEntry> {
    if binding.group != 0 || binding.dynamic_offset {
        return None;
    }
    let ty = match binding.resource {
        BindingResourceKind::StorageBuffer => {
            let read_only = match binding.access.as_str() {
                "read" => true,
                "read_write" => false,
                _ => return None,
            };
            wgpu::BufferBindingType::Storage { read_only }
        }
        BindingResourceKind::UniformBuffer if binding.access == "read" => {
            wgpu::BufferBindingType::Uniform
        }
        _ => return None,
    };
    Some(wgpu::BindGroupLayoutEntry {
        binding: binding.binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: binding.dynamic_offset,
            min_binding_size: NonZeroU64::new(u64::from(binding.minimum_binding_size)),
        },
        count: None,
    })
}

fn reflected_uniform_size(bindings: &[BindingReflection], binding_number: u32) -> Option<u64> {
    bindings
        .iter()
        .find(|binding| {
            binding.binding == binding_number
                && binding.resource == BindingResourceKind::UniformBuffer
        })
        .map(|binding| u64::from(binding.minimum_binding_size))
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
        BasicPointOperation::BasicAdj(parameters) => [
            parameters.black_point,
            parameters.scale,
            parameters.gamma,
            parameters.middle_grey,
            parameters.contrast,
            parameters.hlcomp,
            parameters.hlrange,
            parameters.saturation,
            parameters.vibrance,
        ]
        .iter()
        .any(|value| !value.is_finite()),
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

    #[test]
    fn entry_scoped_runtime_layouts_match_checked_reflection() {
        let registry = ShaderRegistry::try_checked_in().expect("registry");
        for (entry_point, expected_bindings, expects_basic_params) in [
            ("exposure", &[0, 1, 2][..], false),
            ("linear_offset", &[0, 1, 2][..], false),
            ("rgb_gain", &[0, 1, 2][..], false),
            ("basicadj", &[0, 1, 2, 3][..], true),
        ] {
            let entry = registry
                .find("rusttable.point", entry_point)
                .expect("registered point entry");
            let contract =
                point_entry_contract(&registry, entry_point).expect("runtime point contract");
            assert_eq!(contract.source, entry.expanded_source);
            assert_eq!(
                contract
                    .layout_entries
                    .iter()
                    .map(|binding| binding.binding)
                    .collect::<Vec<_>>(),
                expected_bindings
            );
            assert_eq!(
                contract.layout_entries.len(),
                entry.reflection.bindings.len()
            );
            assert_eq!(
                contract.basic_params_size.is_some(),
                expects_basic_params,
                "{entry_point} basicadj parameter allocation"
            );
            for (runtime, reflected) in contract
                .layout_entries
                .iter()
                .zip(&entry.reflection.bindings)
            {
                assert_eq!(runtime.binding, reflected.binding);
                assert_eq!(runtime.visibility, wgpu::ShaderStages::COMPUTE);
                assert_eq!(runtime.count, None);
                let wgpu::BindingType::Buffer {
                    ty,
                    has_dynamic_offset,
                    min_binding_size,
                } = runtime.ty
                else {
                    panic!("point bindings must be buffers");
                };
                let expected_type = match reflected.resource {
                    BindingResourceKind::StorageBuffer => wgpu::BufferBindingType::Storage {
                        read_only: reflected.access == "read",
                    },
                    BindingResourceKind::UniformBuffer => wgpu::BufferBindingType::Uniform,
                    _ => panic!("unsupported reflected point binding"),
                };
                assert_eq!(ty, expected_type);
                assert_eq!(has_dynamic_offset, reflected.dynamic_offset);
                assert_eq!(
                    min_binding_size.map(NonZeroU64::get),
                    Some(u64::from(reflected.minimum_binding_size))
                );
            }
        }
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

    #[tokio::test]
    async fn basicadj_dispatch_preserves_atomic_order_and_alpha_when_gpu_is_available() {
        let Ok(runtime) = GpuRuntime::initialize(crate::GpuRuntimeConfig::default()).await else {
            return;
        };
        if runtime.is_cpu_only() {
            return;
        }
        let parameters = BasicAdjPointParameters {
            black_point: 0.1,
            scale: 1.5,
            gamma: 0.5,
            middle_grey: 0.1842,
            contrast: 1.0,
            hlcomp: 0.0,
            hlrange: 0.9,
            preserve_colors: 1,
            saturation: 0.0,
            vibrance: 0.0,
        };
        let input = [0.4, 0.2, 0.8, 0.37];
        let result = runtime
            .execute_basic_point(BasicPointRequest {
                pixels: &input,
                operations: &[BasicPointOperation::BasicAdj(parameters)],
            })
            .expect("basicadj dispatch");
        let expected = [
            (0.4_f32 - 0.1).mul_add(1.5, 0.0).sqrt(),
            (0.2_f32 - 0.1).mul_add(1.5, 0.0).sqrt(),
            (0.8_f32 - 0.1).mul_add(1.5, 0.0).sqrt(),
            0.37,
        ];
        for (actual, expected) in result.pixels().iter().zip(expected) {
            assert!((actual - expected).abs() < 0.0001, "{actual} != {expected}");
        }
        assert_eq!(result.dispatches(), 1);
    }
}
