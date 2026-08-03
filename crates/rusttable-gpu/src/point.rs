//! WGPU point-operation execution, including direct ports from
//! `data/kernels/{basic.cl::colorcorrection,extended.cl::{colorcontrast,velvia,vibrance}}`
//! and their matching `src/iop/*.c` modules.

use std::fmt;
use std::num::NonZeroU64;

use crate::shader::{BindingReflection, BindingResourceKind, ShaderRegistry};
use crate::{FaultState, GpuRuntime};

const WORKGROUP_SIZE: u32 = 256;
const POINT_PARAMS_SIZE: u64 = 64;
const POINT_PARAMS_BYTES: usize = 64;
const VELVIA_STRENGTH_OFFSET: usize = 56;
const VELVIA_BIAS_OFFSET: usize = 60;
const BASICADJ_PARAMS_SIZE: u64 = 48;
const BASICADJ_PARAMS_BYTES: usize = 48;
const COLORCONTRAST_PARAMS_SIZE: u64 = 32;
const COLORCONTRAST_PARAMS_BYTES: usize = 32;
const VIBRANCE_PARAMS_SIZE: u64 = 16;
const VIBRANCE_PARAMS_BYTES: usize = 16;
const COLORCORRECTION_PARAMS_SIZE: u64 = 32;
const COLORCORRECTION_PARAMS_BYTES: usize = 32;

#[derive(Debug)]
struct PointEntryContract<'a> {
    source: &'a str,
    layout_entries: Vec<wgpu::BindGroupLayoutEntry>,
    params_size: u64,
    basic_params_size: Option<u64>,
    colorcontrast_params_size: Option<u64>,
    vibrance_params_size: Option<u64>,
    colorcorrection_params_size: Option<u64>,
}

struct PointParameterBuffers {
    params: wgpu::Buffer,
    basic_params: Option<wgpu::Buffer>,
    colorcontrast_params: Option<wgpu::Buffer>,
    vibrance_params: Option<wgpu::Buffer>,
    colorcorrection_params: Option<wgpu::Buffer>,
}

/// Explicit channel domain for one shared point-dispatch chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasicPointColorSpace {
    /// Linear-light RGB plus alpha.
    LinearRgb,
    /// Darktable-scale D50 Lab plus alpha: L in 0..100 and a/b centered on zero.
    LabD50,
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

/// One operation in the shared point pipeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BasicPointOperation {
    BasicAdj(BasicAdjPointParameters),
    Exposure {
        stops: f32,
        black: f32,
    },
    LinearOffset {
        value: f32,
    },
    RgbGain {
        red: f32,
        green: f32,
        blue: f32,
    },
    ColorContrast {
        a_steepness: f32,
        a_offset: f32,
        b_steepness: f32,
        b_offset: f32,
        unbound: bool,
    },
    ColorCorrection {
        saturation: f32,
        a_scale: f32,
        a_base: f32,
        b_scale: f32,
        b_base: f32,
    },
    Vibrance {
        /// Native normalized amount (`params.amount / 100.0`).
        amount: f32,
    },
    Velvia {
        strength: f32,
        bias: f32,
    },
}

impl BasicPointOperation {
    const fn entry_point(self) -> &'static str {
        match self {
            Self::BasicAdj(_) => "basicadj",
            Self::Exposure { .. } => "exposure",
            Self::LinearOffset { .. } => "linear_offset",
            Self::RgbGain { .. } => "rgb_gain",
            Self::ColorContrast { .. } => "colorcontrast",
            Self::ColorCorrection { .. } => "colorcorrection",
            Self::Vibrance { .. } => "vibrance",
            Self::Velvia { .. } => "velvia",
        }
    }

    /// Returns the channel domain required by this operation.
    #[must_use]
    pub const fn required_color_space(self) -> BasicPointColorSpace {
        match self {
            Self::ColorContrast { .. } | Self::ColorCorrection { .. } | Self::Vibrance { .. } => {
                BasicPointColorSpace::LabD50
            }
            Self::BasicAdj(_)
            | Self::Exposure { .. }
            | Self::LinearOffset { .. }
            | Self::RgbGain { .. }
            | Self::Velvia { .. } => BasicPointColorSpace::LinearRgb,
        }
    }

    fn params(self, pixel_count: u32) -> [u8; POINT_PARAMS_BYTES] {
        let mut bytes = [0_u8; POINT_PARAMS_BYTES];
        bytes[0..4].copy_from_slice(&pixel_count.to_le_bytes());
        let values = match self {
            Self::BasicAdj(_)
            | Self::ColorContrast { .. }
            | Self::ColorCorrection { .. }
            | Self::Vibrance { .. }
            | Self::Velvia { .. } => [0.0, 0.0, 1.0, 1.0, 1.0, 2.2, 0.0],
            Self::Exposure { stops, black } => [stops, 0.0, 1.0, 1.0, 1.0, 2.2, black],
            Self::LinearOffset { value } => [0.0, value, 1.0, 1.0, 1.0, 2.2, 0.0],
            Self::RgbGain { red, green, blue } => [0.0, 0.0, red, green, blue, 2.2, 0.0],
        };
        for (index, value) in values.into_iter().enumerate() {
            let offset = 28 + index * 4;
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        if let Self::Velvia { strength, bias } = self {
            bytes[VELVIA_STRENGTH_OFFSET..VELVIA_BIAS_OFFSET]
                .copy_from_slice(&strength.to_le_bytes());
            bytes[VELVIA_BIAS_OFFSET..POINT_PARAMS_BYTES].copy_from_slice(&bias.to_le_bytes());
        }
        bytes
    }

    fn vibrance_params(self) -> [u8; VIBRANCE_PARAMS_BYTES] {
        let mut bytes = [0_u8; VIBRANCE_PARAMS_BYTES];
        let Self::Vibrance { amount } = self else {
            return bytes;
        };
        bytes[0..4].copy_from_slice(&amount.to_le_bytes());
        bytes
    }

    fn colorcontrast_params(self) -> [u8; COLORCONTRAST_PARAMS_BYTES] {
        let mut bytes = [0_u8; COLORCONTRAST_PARAMS_BYTES];
        let Self::ColorContrast {
            a_steepness,
            a_offset,
            b_steepness,
            b_offset,
            unbound,
        } = self
        else {
            return bytes;
        };
        for (index, value) in [a_steepness, a_offset, b_steepness, b_offset]
            .into_iter()
            .enumerate()
        {
            let offset = index * 4;
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[16..20].copy_from_slice(&u32::from(unbound).to_le_bytes());
        bytes
    }

    fn colorcorrection_params(self) -> [u8; COLORCORRECTION_PARAMS_BYTES] {
        let mut bytes = [0_u8; COLORCORRECTION_PARAMS_BYTES];
        let Self::ColorCorrection {
            saturation,
            a_scale,
            a_base,
            b_scale,
            b_base,
        } = self
        else {
            return bytes;
        };
        for (index, value) in [saturation, a_scale, a_base, b_scale, b_base]
            .into_iter()
            .enumerate()
        {
            let offset = index * 4;
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

    fn parameters_are_finite(self) -> bool {
        match self {
            Self::BasicAdj(parameters) => [
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
            .all(|value| value.is_finite()),
            Self::Exposure { stops, black } => stops.is_finite() && black.is_finite(),
            Self::LinearOffset { value } => value.is_finite(),
            Self::RgbGain { red, green, blue } => {
                red.is_finite() && green.is_finite() && blue.is_finite()
            }
            Self::ColorContrast {
                a_steepness,
                a_offset,
                b_steepness,
                b_offset,
                ..
            } => [a_steepness, a_offset, b_steepness, b_offset]
                .iter()
                .all(|value| value.is_finite()),
            Self::ColorCorrection {
                saturation,
                a_scale,
                a_base,
                b_scale,
                b_base,
            } => [saturation, a_scale, a_base, b_scale, b_base]
                .iter()
                .all(|value| value.is_finite()),
            Self::Vibrance { amount } => amount.is_finite(),
            Self::Velvia { strength, bias } => strength.is_finite() && bias.is_finite(),
        }
    }
}

/// A checked request for the WGPU point-operation executor.
#[derive(Debug, Clone, Copy)]
pub struct BasicPointRequest<'a> {
    /// Packed four-channel f32 pixels. Alpha is copied unchanged.
    pub pixels: &'a [f32],
    pub operations: &'a [BasicPointOperation],
    /// Proven channel domain for every operation in this dispatch chain.
    pub color_space: BasicPointColorSpace,
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
    NonFiniteInput {
        component: usize,
    },
    NonFiniteParameter,
    ColorSpaceMismatch {
        requested: BasicPointColorSpace,
        required: BasicPointColorSpace,
    },
    ColorSpaceBoundaryUnavailable {
        required: BasicPointColorSpace,
    },
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
            Self::ColorSpaceMismatch {
                requested,
                required,
            } => write!(
                formatter,
                "basic WGPU point operation requires {required:?}, not requested {requested:?}"
            ),
            Self::ColorSpaceBoundaryUnavailable { required } => write!(
                formatter,
                "basic WGPU point chain does not expose the isolated {required:?} boundary required by the operation"
            ),
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
    /// Transfer and Lab conversion remain at the typed pixelpipe boundary. The
    /// executor receives a chain whose explicit channel domain matches every
    /// operation and returns that same representation, preserving alpha.
    /// Unsupported transfer, opacity, masks, mixed-domain chains, and non-point
    /// stages are rejected by the pixelpipe adapter before this method is called.
    #[expect(
        clippy::too_many_lines,
        reason = "the point GPU path preserves operation order, buffer ABI, dispatch, and readback"
    )]
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
                let colorcontrast_params = contract.colorcontrast_params_size.map(|size| {
                    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("RustTable color contrast parameters"),
                        size,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                    queue.write_buffer(&buffer, 0, &operation.colorcontrast_params());
                    buffer
                });
                let vibrance_params = contract.vibrance_params_size.map(|size| {
                    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("RustTable Vibrance parameters"),
                        size,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                    queue.write_buffer(&buffer, 0, &operation.vibrance_params());
                    buffer
                });
                let colorcorrection_params = contract.colorcorrection_params_size.map(|size| {
                    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("RustTable color correction parameters"),
                        size,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                    queue.write_buffer(&buffer, 0, &operation.colorcorrection_params());
                    buffer
                });
                PointParameterBuffers {
                    params,
                    basic_params,
                    colorcontrast_params,
                    vibrance_params,
                    colorcorrection_params,
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
            if let Some(colorcontrast_params) = &parameter_buffers.colorcontrast_params {
                bind_group_entries.push(wgpu::BindGroupEntry {
                    binding: 4,
                    resource: colorcontrast_params.as_entire_binding(),
                });
            }
            if let Some(vibrance_params) = &parameter_buffers.vibrance_params {
                bind_group_entries.push(wgpu::BindGroupEntry {
                    binding: 5,
                    resource: vibrance_params.as_entire_binding(),
                });
            }
            if let Some(colorcorrection_params) = &parameter_buffers.colorcorrection_params {
                bind_group_entries.push(wgpu::BindGroupEntry {
                    binding: 6,
                    resource: colorcorrection_params.as_entire_binding(),
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
    let expected_bindings: &[u32] = match entry_point {
        "basicadj" => &[0, 1, 2, 3],
        "colorcontrast" => &[0, 1, 2, 4],
        "vibrance" => &[0, 1, 2, 5],
        "colorcorrection" => &[0, 1, 2, 6],
        _ => &[0, 1, 2],
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
    let colorcontrast_params_size = reflected_uniform_size(&reflection.bindings, 4);
    if colorcontrast_params_size.is_some() != (entry_point == "colorcontrast")
        || colorcontrast_params_size.is_some_and(|size| size != COLORCONTRAST_PARAMS_SIZE)
    {
        return Err(BasicPointError::ShaderUnavailable);
    }
    let vibrance_params_size = reflected_uniform_size(&reflection.bindings, 5);
    if vibrance_params_size.is_some() != (entry_point == "vibrance")
        || vibrance_params_size.is_some_and(|size| size != VIBRANCE_PARAMS_SIZE)
    {
        return Err(BasicPointError::ShaderUnavailable);
    }
    let colorcorrection_params_size = reflected_uniform_size(&reflection.bindings, 6);
    if colorcorrection_params_size.is_some() != (entry_point == "colorcorrection")
        || colorcorrection_params_size.is_some_and(|size| size != COLORCORRECTION_PARAMS_SIZE)
    {
        return Err(BasicPointError::ShaderUnavailable);
    }
    Ok(PointEntryContract {
        source: &entry.expanded_source,
        layout_entries,
        params_size,
        basic_params_size,
        colorcontrast_params_size,
        vibrance_params_size,
        colorcorrection_params_size,
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
    if request
        .operations
        .iter()
        .any(|operation| matches!(operation, BasicPointOperation::BasicAdj(_)))
    {
        // The Basic Adjust shader has no authoritative working-profile or LUT
        // payload. Never execute its legacy camera-luminance shortcut.
        return Err(BasicPointError::ShaderUnavailable);
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
    if request
        .operations
        .iter()
        .any(|operation| !operation.parameters_are_finite())
    {
        return Err(BasicPointError::NonFiniteParameter);
    }
    validate_operation_color_space(request)?;
    let workgroups = request.pixels.len().div_ceil(4 * WORKGROUP_SIZE as usize) as u64;
    if workgroups > u64::from(runtime.snapshot().limits.max_workgroups_per_dimension) {
        return Err(BasicPointError::TooManyWorkgroups);
    }
    Ok(())
}

fn validate_operation_color_space(request: BasicPointRequest<'_>) -> Result<(), BasicPointError> {
    if let Some(required) = request
        .operations
        .iter()
        .map(|operation| operation.required_color_space())
        .find(|required| *required != request.color_space)
    {
        return Err(BasicPointError::ColorSpaceMismatch {
            requested: request.color_space,
            required,
        });
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
    #![expect(
        clippy::suboptimal_flops,
        reason = "Reference tests preserve native arithmetic operation order."
    )]

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
    fn velvia_parameters_use_the_final_reserved_uniform_words() {
        let bytes = BasicPointOperation::Velvia {
            strength: 0.25,
            bias: 0.75,
        }
        .params(11);
        let established = BasicPointOperation::LinearOffset { value: 0.0 }.params(11);
        assert_eq!(&bytes[0..4], &11_u32.to_le_bytes());
        assert_eq!(
            &bytes[..VELVIA_STRENGTH_OFFSET],
            &established[..VELVIA_STRENGTH_OFFSET],
            "Velvia must not disturb the established point uniform fields"
        );
        assert_eq!(
            &bytes[VELVIA_STRENGTH_OFFSET..VELVIA_BIAS_OFFSET],
            &0.25_f32.to_le_bytes()
        );
        assert_eq!(
            &bytes[VELVIA_BIAS_OFFSET..POINT_PARAMS_BYTES],
            &0.75_f32.to_le_bytes()
        );
        assert_eq!(bytes.len(), POINT_PARAMS_BYTES);
    }

    #[test]
    fn velvia_parameter_validation_rejects_each_non_finite_scalar() {
        for operation in [
            BasicPointOperation::Velvia {
                strength: f32::NAN,
                bias: 1.0,
            },
            BasicPointOperation::Velvia {
                strength: 0.25,
                bias: f32::INFINITY,
            },
        ] {
            assert!(!operation.parameters_are_finite());
        }
        assert!(
            BasicPointOperation::Velvia {
                strength: 0.25,
                bias: 1.0,
            }
            .parameters_are_finite()
        );
    }

    #[test]
    fn colorcontrast_parameters_follow_the_dedicated_uniform_layout() {
        let operation = BasicPointOperation::ColorContrast {
            a_steepness: 1.25,
            a_offset: -2.5,
            b_steepness: 0.75,
            b_offset: 3.5,
            unbound: true,
        };
        let bytes = operation.colorcontrast_params();
        for (index, expected) in [1.25_f32, -2.5, 0.75, 3.5].into_iter().enumerate() {
            let offset = index * 4;
            assert_eq!(&bytes[offset..offset + 4], &expected.to_le_bytes());
        }
        assert_eq!(&bytes[16..20], &1_u32.to_le_bytes());
        assert!(bytes[20..].iter().all(|byte| *byte == 0));
        assert_eq!(bytes.len(), COLORCONTRAST_PARAMS_BYTES);
    }

    #[test]
    fn colorcorrection_parameters_follow_the_native_dedicated_uniform_layout() {
        let operation = BasicPointOperation::ColorCorrection {
            saturation: 1.25,
            a_scale: -0.125,
            a_base: 3.5,
            b_scale: 0.25,
            b_base: -4.5,
        };
        let bytes = operation.colorcorrection_params();
        for (index, expected) in [1.25_f32, -0.125, 3.5, 0.25, -4.5].into_iter().enumerate() {
            let offset = index * 4;
            assert_eq!(&bytes[offset..offset + 4], &expected.to_le_bytes());
        }
        assert!(bytes[20..].iter().all(|byte| *byte == 0));
        assert_eq!(bytes.len(), COLORCORRECTION_PARAMS_BYTES);
    }

    #[test]
    fn vibrance_parameters_follow_the_dedicated_uniform_layout() {
        let operation = BasicPointOperation::Vibrance { amount: 0.25 };
        let bytes = operation.vibrance_params();
        assert_eq!(&bytes[0..4], &0.25_f32.to_le_bytes());
        assert!(bytes[4..].iter().all(|byte| *byte == 0));
        assert_eq!(bytes.len(), VIBRANCE_PARAMS_BYTES);
    }

    #[test]
    fn vibrance_rejects_non_finite_amount_and_requires_lab_d50() {
        assert!(!BasicPointOperation::Vibrance { amount: f32::NAN }.parameters_are_finite());
        assert!(
            !BasicPointOperation::Vibrance {
                amount: f32::INFINITY,
            }
            .parameters_are_finite()
        );
        let operation = BasicPointOperation::Vibrance { amount: 0.25 };
        assert!(operation.parameters_are_finite());
        assert_eq!(
            operation.required_color_space(),
            BasicPointColorSpace::LabD50
        );
        assert_eq!(
            validate_operation_color_space(BasicPointRequest {
                pixels: &[50.0, 10.0, -20.0, 1.0],
                operations: &[operation],
                color_space: BasicPointColorSpace::LinearRgb,
            }),
            Err(BasicPointError::ColorSpaceMismatch {
                requested: BasicPointColorSpace::LinearRgb,
                required: BasicPointColorSpace::LabD50,
            })
        );
    }

    #[test]
    fn colorcontrast_rejects_non_finite_parameters_and_requires_lab_d50() {
        for operation in [
            BasicPointOperation::ColorContrast {
                a_steepness: f32::NAN,
                a_offset: 0.0,
                b_steepness: 1.0,
                b_offset: 0.0,
                unbound: true,
            },
            BasicPointOperation::ColorContrast {
                a_steepness: 1.0,
                a_offset: 0.0,
                b_steepness: f32::INFINITY,
                b_offset: 0.0,
                unbound: false,
            },
        ] {
            assert!(!operation.parameters_are_finite());
        }
        let operation = BasicPointOperation::ColorContrast {
            a_steepness: 1.0,
            a_offset: 0.0,
            b_steepness: 1.0,
            b_offset: 0.0,
            unbound: true,
        };
        assert!(operation.parameters_are_finite());
        assert_eq!(
            operation.required_color_space(),
            BasicPointColorSpace::LabD50
        );
        assert_eq!(
            validate_operation_color_space(BasicPointRequest {
                pixels: &[50.0, 0.0, 0.0, 1.0],
                operations: &[operation],
                color_space: BasicPointColorSpace::LinearRgb,
            }),
            Err(BasicPointError::ColorSpaceMismatch {
                requested: BasicPointColorSpace::LinearRgb,
                required: BasicPointColorSpace::LabD50,
            })
        );
    }

    #[test]
    fn colorcorrection_rejects_non_finite_parameters_and_requires_lab_d50() {
        for operation in [
            BasicPointOperation::ColorCorrection {
                saturation: f32::NAN,
                a_scale: 0.0,
                a_base: 0.0,
                b_scale: 0.0,
                b_base: 0.0,
            },
            BasicPointOperation::ColorCorrection {
                saturation: 1.0,
                a_scale: 0.0,
                a_base: 0.0,
                b_scale: f32::INFINITY,
                b_base: 0.0,
            },
        ] {
            assert!(!operation.parameters_are_finite());
        }
        let operation = BasicPointOperation::ColorCorrection {
            saturation: 1.0,
            a_scale: -0.025,
            a_base: 3.0,
            b_scale: 0.075,
            b_base: -2.0,
        };
        assert!(operation.parameters_are_finite());
        assert_eq!(
            operation.required_color_space(),
            BasicPointColorSpace::LabD50
        );
        assert_eq!(
            validate_operation_color_space(BasicPointRequest {
                pixels: &[50.0, 0.0, 0.0, 1.0],
                operations: &[operation],
                color_space: BasicPointColorSpace::LinearRgb,
            }),
            Err(BasicPointError::ColorSpaceMismatch {
                requested: BasicPointColorSpace::LinearRgb,
                required: BasicPointColorSpace::LabD50,
            })
        );
    }

    #[expect(
        clippy::too_many_lines,
        reason = "Keep the complete checked-reflection layout matrix and per-binding assertions together."
    )]
    #[test]
    fn entry_scoped_runtime_layouts_match_checked_reflection() {
        let registry = ShaderRegistry::try_checked_in().expect("registry");
        for (
            entry_point,
            expected_bindings,
            expects_basic_params,
            expects_colorcontrast_params,
            expects_vibrance_params,
            expects_colorcorrection_params,
        ) in [
            ("exposure", &[0, 1, 2][..], false, false, false, false),
            ("linear_offset", &[0, 1, 2][..], false, false, false, false),
            ("rgb_gain", &[0, 1, 2][..], false, false, false, false),
            ("velvia", &[0, 1, 2][..], false, false, false, false),
            ("basicadj", &[0, 1, 2, 3][..], true, false, false, false),
            (
                "colorcontrast",
                &[0, 1, 2, 4][..],
                false,
                true,
                false,
                false,
            ),
            ("vibrance", &[0, 1, 2, 5][..], false, false, true, false),
            (
                "colorcorrection",
                &[0, 1, 2, 6][..],
                false,
                false,
                false,
                true,
            ),
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
            assert_eq!(
                contract.colorcontrast_params_size.is_some(),
                expects_colorcontrast_params,
                "{entry_point} color contrast parameter allocation"
            );
            assert_eq!(
                contract.vibrance_params_size.is_some(),
                expects_vibrance_params,
                "{entry_point} Vibrance parameter allocation"
            );
            assert_eq!(
                contract.colorcorrection_params_size.is_some(),
                expects_colorcorrection_params,
                "{entry_point} color correction parameter allocation"
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
                color_space: BasicPointColorSpace::LinearRgb,
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
                color_space: BasicPointColorSpace::LinearRgb,
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
    async fn basicadj_gpu_dispatch_fails_closed_without_profile_evidence() {
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
        let error = runtime
            .execute_basic_point(BasicPointRequest {
                pixels: &input,
                operations: &[BasicPointOperation::BasicAdj(parameters)],
                color_space: BasicPointColorSpace::LinearRgb,
            })
            .expect_err("Basic Adjust GPU execution must remain unavailable");
        assert!(matches!(error, BasicPointError::ShaderUnavailable));
    }

    fn comparison_clamps(value: f32, low: f32, high: f32) -> f32 {
        if value > low {
            if value < high { value } else { high }
        } else {
            low
        }
    }

    #[test]
    fn velvia_comparison_clamp_routes_nan_to_the_lower_bound() {
        assert_eq!(comparison_clamps(f32::NAN, 0.0, 1.0).to_bits(), 0);
        assert_eq!(
            comparison_clamps(f32::INFINITY, 0.0, 1.0).to_bits(),
            1.0_f32.to_bits()
        );
    }

    #[expect(
        clippy::manual_midpoint,
        reason = "the parity reference preserves Darktable's overflow-sensitive expression order"
    )]
    fn velvia_cpu(pixel: [f32; 4], strength: f32, bias: f32) -> [f32; 4] {
        if strength <= 0.0 {
            return pixel;
        }
        let pmax = pixel[0].max(pixel[1].max(pixel[2]));
        let pmin = pixel[0].min(pixel[1].min(pixel[2]));
        let plum = (pmax + pmin) / 2.0;
        let psat = if plum <= 0.5 {
            (pmax - pmin) / ((0.00001 + pmax) + pmin)
        } else {
            (pmax - pmin) / (0.00001 + 0.0_f32.max((2.0 - pmax) - pmin))
        };
        let pweight = comparison_clamps(
            ((1.0 - (1.5 * psat)) + ((1.0 + (plum - 0.5).abs() * 2.0) * (1.0 - bias)))
                / (1.0 + (1.0 - bias)),
            0.0,
            1.0,
        );
        let saturation = strength * pweight;
        [
            comparison_clamps(
                pixel[0] + saturation * (pixel[0] - 0.5 * (pixel[1] + pixel[2])),
                0.0,
                1.0,
            ),
            comparison_clamps(
                pixel[1] + saturation * (pixel[1] - 0.5 * (pixel[2] + pixel[0])),
                0.0,
                1.0,
            ),
            comparison_clamps(
                pixel[2] + saturation * (pixel[2] - 0.5 * (pixel[0] + pixel[1])),
                0.0,
                1.0,
            ),
            pixel[3],
        ]
    }

    fn colorcontrast_cpu(
        pixel: [f32; 4],
        a_steepness: f32,
        a_offset: f32,
        b_steepness: f32,
        b_offset: f32,
        unbound: bool,
    ) -> [f32; 4] {
        let a = pixel[1] * a_steepness + a_offset;
        let b = pixel[2] * b_steepness + b_offset;
        [
            pixel[0],
            if unbound {
                a
            } else {
                comparison_clamps(a, -128.0, 128.0)
            },
            if unbound {
                b
            } else {
                comparison_clamps(b, -128.0, 128.0)
            },
            pixel[3],
        ]
    }

    fn colorcorrection_cpu(
        pixel: [f32; 4],
        saturation: f32,
        a_scale: f32,
        a_base: f32,
        b_scale: f32,
        b_base: f32,
    ) -> [f32; 4] {
        [
            pixel[0],
            saturation * (pixel[1] + pixel[0] * a_scale + a_base),
            saturation * (pixel[2] + pixel[0] * b_scale + b_base),
            pixel[3],
        ]
    }

    fn opencl_default_hypot(x: f32, y: f32) -> f32 {
        let absolute_x = x.abs();
        let absolute_y = y.abs();
        let maximum = absolute_x.max(absolute_y);
        if maximum == 0.0 {
            return 0.0;
        }
        let ratio = absolute_x.min(absolute_y) / maximum;
        maximum * (1.0 + ratio * ratio).sqrt()
    }

    fn vibrance_opencl_default(pixel: [f32; 4], amount: f32) -> [f32; 4] {
        let sw = opencl_default_hypot(pixel[1], pixel[2]) / 256.0;
        let lightness_scale = 1.0 - amount * sw * 0.25;
        let saturation_scale = 1.0 + amount * sw;
        [
            pixel[0] * lightness_scale,
            pixel[1] * saturation_scale,
            pixel[2] * saturation_scale,
            pixel[3],
        ]
    }

    #[test]
    fn default_opencl_hypot_avoids_intermediate_overflow_until_the_true_norm_overflows() {
        assert_eq!(
            opencl_default_hypot(f32::MAX, 1.0).to_bits(),
            f32::MAX.to_bits()
        );
        assert!(opencl_default_hypot(f32::MAX, f32::MAX).is_infinite());
    }

    #[tokio::test]
    async fn vibrance_dispatch_matches_default_opencl_lab_formula_without_clamping() {
        let Ok(runtime) = GpuRuntime::initialize(crate::GpuRuntimeConfig::default()).await else {
            return;
        };
        if runtime.is_cpu_only() {
            return;
        }
        let corpus = [
            [50.0, 10.0, -20.0, 0.0],
            [80.0, 100.0, -100.0, 0.37],
            [0.0, -128.0, 128.0, 1.0],
            [100.0, 0.0, 0.0, f32::from_bits(1)],
        ];
        let input = corpus.into_iter().flatten().collect::<Vec<_>>();
        let result = runtime
            .execute_basic_point(BasicPointRequest {
                pixels: &input,
                operations: &[BasicPointOperation::Vibrance { amount: 1.0 }],
                color_space: BasicPointColorSpace::LabD50,
            })
            .expect("Vibrance point dispatch");
        let (actual_pixels, remainder) = result.pixels().as_chunks::<4>();
        assert!(remainder.is_empty());
        for (index, (actual, source)) in actual_pixels.iter().zip(corpus).enumerate() {
            let expected = vibrance_opencl_default(source, 1.0);
            for channel in 0..3 {
                assert!(
                    (actual[channel] - expected[channel]).abs() <= 0.000_02,
                    "pixel {index} channel {channel}: {} != {}",
                    actual[channel],
                    expected[channel]
                );
            }
            assert_eq!(
                actual[3].to_bits(),
                expected[3].to_bits(),
                "pixel {index} alpha"
            );
        }
        assert!(
            actual_pixels[2][1].abs() > 128.0,
            "native Vibrance does not clamp Lab chroma"
        );
        assert_eq!(result.dispatches(), 1);
    }

    #[expect(
        clippy::too_many_lines,
        reason = "Keep the complete GPU overflow matrix, including signed-zero and boundary cases, together."
    )]
    #[tokio::test]
    async fn vibrance_dispatch_overflows_when_the_true_opencl_hypot_exceeds_f32() {
        let Ok(runtime) = GpuRuntime::initialize(crate::GpuRuntimeConfig::default()).await else {
            return;
        };
        if runtime.is_cpu_only() {
            return;
        }
        let input = [50.0, f32::MAX, -f32::MAX, f32::from_bits(0x3eaa_aaab)];
        let result = runtime
            .execute_basic_point(BasicPointRequest {
                pixels: &input,
                operations: &[BasicPointOperation::Vibrance { amount: 1.0 }],
                color_space: BasicPointColorSpace::LabD50,
            })
            .expect("extreme-finite Vibrance point dispatch");
        assert!(
            result.pixels()[0].is_infinite(),
            "extreme Vibrance result: {:?}",
            result.pixels()
        );
        assert!(result.pixels()[0].is_sign_negative());
        assert!(result.pixels()[1].is_infinite());
        assert!(result.pixels()[1].is_sign_positive());
        assert!(result.pixels()[2].is_infinite());
        assert!(result.pixels()[2].is_sign_negative());
        assert_eq!(result.pixels()[3].to_bits(), input[3].to_bits());
        assert_eq!(result.dispatches(), 1);

        let negative = runtime
            .execute_basic_point(BasicPointRequest {
                pixels: &input,
                operations: &[BasicPointOperation::Vibrance { amount: -1.0 }],
                color_space: BasicPointColorSpace::LabD50,
            })
            .expect("negative extreme-finite Vibrance point dispatch");
        assert!(negative.pixels()[0].is_infinite());
        assert!(negative.pixels()[0].is_sign_positive());
        assert!(negative.pixels()[1].is_infinite());
        assert!(negative.pixels()[1].is_sign_negative());
        assert!(negative.pixels()[2].is_infinite());
        assert!(negative.pixels()[2].is_sign_positive());
        assert_eq!(negative.pixels()[3].to_bits(), input[3].to_bits());

        let zero = runtime
            .execute_basic_point(BasicPointRequest {
                pixels: &input,
                operations: &[BasicPointOperation::Vibrance { amount: 0.0 }],
                color_space: BasicPointColorSpace::LabD50,
            })
            .expect("zero extreme-finite Vibrance point dispatch");
        assert!(
            zero.pixels()[..3].iter().all(|channel| channel.is_nan()),
            "OpenCL zero times an infinite true chroma norm is NaN: {:?}",
            zero.pixels()
        );
        assert_eq!(zero.pixels()[3].to_bits(), input[3].to_bits());

        let negative_zero = runtime
            .execute_basic_point(BasicPointRequest {
                pixels: &input,
                operations: &[BasicPointOperation::Vibrance { amount: -0.0 }],
                color_space: BasicPointColorSpace::LabD50,
            })
            .expect("negative-zero extreme-finite Vibrance point dispatch");
        assert!(
            negative_zero.pixels()[..3]
                .iter()
                .all(|channel| channel.is_nan()),
            "negative zero times an infinite true chroma norm is NaN: {:?}",
            negative_zero.pixels()
        );
        assert_eq!(negative_zero.pixels()[3].to_bits(), input[3].to_bits());

        let zero_lightness = [
            0.0,
            f32::MAX,
            f32::MAX,
            f32::from_bits(1),
            -0.0,
            f32::MAX,
            -f32::MAX,
            f32::from_bits(2),
        ];
        let zero_lightness_result = runtime
            .execute_basic_point(BasicPointRequest {
                pixels: &zero_lightness,
                operations: &[BasicPointOperation::Vibrance { amount: 1.0 }],
                color_space: BasicPointColorSpace::LabD50,
            })
            .expect("zero-lightness extreme-finite Vibrance point dispatch");
        let (zero_lightness_pixels, remainder) = zero_lightness_result.pixels().as_chunks::<4>();
        assert!(remainder.is_empty());
        assert!(zero_lightness_pixels[0][0].is_nan());
        assert!(zero_lightness_pixels[1][0].is_nan());
        for channel in [
            zero_lightness_pixels[0][1],
            zero_lightness_pixels[0][2],
            zero_lightness_pixels[1][1],
            zero_lightness_pixels[1][2],
        ] {
            assert!(channel.is_infinite());
        }
        assert!(zero_lightness_pixels[0][1].is_sign_positive());
        assert!(zero_lightness_pixels[0][2].is_sign_positive());
        assert!(zero_lightness_pixels[1][1].is_sign_positive());
        assert!(zero_lightness_pixels[1][2].is_sign_negative());
        assert_eq!(
            zero_lightness_pixels[0][3].to_bits(),
            zero_lightness[3].to_bits()
        );
        assert_eq!(
            zero_lightness_pixels[1][3].to_bits(),
            zero_lightness[7].to_bits()
        );

        let near_boundary = [
            50.0,
            f32::MAX,
            f32::from_bits(0x7957_44fd),
            f32::from_bits(0x3eaa_aaab),
        ];
        let near_boundary_result = runtime
            .execute_basic_point(BasicPointRequest {
                pixels: &near_boundary,
                operations: &[BasicPointOperation::Vibrance { amount: 1.0 }],
                color_space: BasicPointColorSpace::LabD50,
            })
            .expect("near-boundary Vibrance point dispatch");
        assert!(
            near_boundary_result.pixels()[0].is_infinite(),
            "the declared WGSL-backend overflow boundary remains stable: {:?}",
            near_boundary_result.pixels()
        );
    }

    #[tokio::test]
    async fn zero_amount_vibrance_is_identity_when_only_the_naive_square_would_overflow() {
        let Ok(runtime) = GpuRuntime::initialize(crate::GpuRuntimeConfig::default()).await else {
            return;
        };
        if runtime.is_cpu_only() {
            return;
        }
        let input = [50.0, f32::MAX, 1.0, f32::from_bits(0x3eaa_aaab)];
        let result = runtime
            .execute_basic_point(BasicPointRequest {
                pixels: &input,
                operations: &[BasicPointOperation::Vibrance { amount: 0.0 }],
                color_space: BasicPointColorSpace::LabD50,
            })
            .expect("default non-fast Vibrance point dispatch");
        assert_eq!(
            result
                .pixels()
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>(),
            input
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>()
        );
        assert_eq!(result.dispatches(), 1);
    }

    #[tokio::test]
    async fn vibrance_dispatch_composes_with_colorcontrast_in_authored_lab_order() {
        let Ok(runtime) = GpuRuntime::initialize(crate::GpuRuntimeConfig::default()).await else {
            return;
        };
        if runtime.is_cpu_only() {
            return;
        }
        let source = [50.0, 10.0, -20.0, 0.37];
        let operations = [
            BasicPointOperation::ColorContrast {
                a_steepness: 2.0,
                a_offset: 3.0,
                b_steepness: 1.5,
                b_offset: -4.0,
                unbound: true,
            },
            BasicPointOperation::Vibrance { amount: 0.25 },
            BasicPointOperation::ColorContrast {
                a_steepness: 0.5,
                a_offset: -2.0,
                b_steepness: 2.0,
                b_offset: 5.0,
                unbound: false,
            },
        ];
        let result = runtime
            .execute_basic_point(BasicPointRequest {
                pixels: &source,
                operations: &operations,
                color_space: BasicPointColorSpace::LabD50,
            })
            .expect("mixed Lab point dispatch");
        let first = colorcontrast_cpu(source, 2.0, 3.0, 1.5, -4.0, true);
        let second = vibrance_opencl_default(first, 0.25);
        let expected = colorcontrast_cpu(second, 0.5, -2.0, 2.0, 5.0, false);
        for (channel, (actual, expected)) in
            result.pixels().iter().copied().zip(expected).enumerate()
        {
            if channel == 3 {
                assert_eq!(actual.to_bits(), expected.to_bits(), "alpha");
            } else {
                assert!(
                    (actual - expected).abs() <= 0.000_02,
                    "channel {channel}: {actual} != {expected}"
                );
            }
        }
        assert_eq!(result.dispatches(), 3);
    }

    #[tokio::test]
    async fn zero_amount_vibrance_is_bit_exact_identity() {
        let Ok(runtime) = GpuRuntime::initialize(crate::GpuRuntimeConfig::default()).await else {
            return;
        };
        if runtime.is_cpu_only() {
            return;
        }
        let input = [
            50.0,
            -128.0,
            128.0,
            f32::from_bits(1),
            0.0,
            0.0,
            0.0,
            f32::from_bits(0x3eaa_aaab),
        ];
        let result = runtime
            .execute_basic_point(BasicPointRequest {
                pixels: &input,
                operations: &[BasicPointOperation::Vibrance { amount: 0.0 }],
                color_space: BasicPointColorSpace::LabD50,
            })
            .expect("zero-amount Vibrance point dispatch");
        assert_eq!(
            result
                .pixels()
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>(),
            input
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>()
        );
        assert_eq!(result.dispatches(), 1);
    }

    #[tokio::test]
    async fn colorcorrection_dispatch_matches_native_lab_formula_without_clamping() {
        let Ok(runtime) = GpuRuntime::initialize(crate::GpuRuntimeConfig::default()).await else {
            return;
        };
        if runtime.is_cpu_only() {
            return;
        }
        let corpus = [
            [50.0, 10.0, -20.0, 0.0],
            [80.0, 100.0, -100.0, 0.37],
            [0.0, -128.0, 128.0, 1.0],
            [100.0, 0.0, 0.0, f32::from_bits(1)],
        ];
        let input = corpus.into_iter().flatten().collect::<Vec<_>>();
        let result = runtime
            .execute_basic_point(BasicPointRequest {
                pixels: &input,
                operations: &[BasicPointOperation::ColorCorrection {
                    saturation: 1.5,
                    a_scale: 0.25,
                    a_base: 3.0,
                    b_scale: -0.125,
                    b_base: -4.0,
                }],
                color_space: BasicPointColorSpace::LabD50,
            })
            .expect("Color Correction point dispatch");
        let (actual_pixels, remainder) = result.pixels().as_chunks::<4>();
        assert!(remainder.is_empty());
        for (index, (actual, source)) in actual_pixels.iter().zip(corpus).enumerate() {
            let expected = colorcorrection_cpu(source, 1.5, 0.25, 3.0, -0.125, -4.0);
            for channel in 0..3 {
                assert!(
                    (actual[channel] - expected[channel]).abs() <= 0.000_02,
                    "pixel {index} channel {channel}: {} != {}",
                    actual[channel],
                    expected[channel]
                );
            }
            assert_eq!(
                actual[3].to_bits(),
                expected[3].to_bits(),
                "pixel {index} alpha"
            );
        }
        assert!(
            actual_pixels[1][1].abs() > 128.0,
            "native Color Correction does not clamp Lab chroma"
        );
        assert_eq!(result.dispatches(), 1);
    }

    #[tokio::test]
    async fn colorcorrection_dispatch_composes_with_colorcontrast_and_vibrance_in_authored_order() {
        let Ok(runtime) = GpuRuntime::initialize(crate::GpuRuntimeConfig::default()).await else {
            return;
        };
        if runtime.is_cpu_only() {
            return;
        }
        let source = [50.0, 10.0, -20.0, f32::from_bits(0x3eaa_aaab)];
        let operations = [
            BasicPointOperation::ColorCorrection {
                saturation: 1.25,
                a_scale: 0.1,
                a_base: 3.0,
                b_scale: -0.05,
                b_base: -4.0,
            },
            BasicPointOperation::ColorContrast {
                a_steepness: 1.5,
                a_offset: -2.0,
                b_steepness: 0.75,
                b_offset: 5.0,
                unbound: true,
            },
            BasicPointOperation::Vibrance { amount: 0.25 },
        ];
        let result = runtime
            .execute_basic_point(BasicPointRequest {
                pixels: &source,
                operations: &operations,
                color_space: BasicPointColorSpace::LabD50,
            })
            .expect("mixed Color Correction Lab point dispatch");
        let corrected = colorcorrection_cpu(source, 1.25, 0.1, 3.0, -0.05, -4.0);
        let contrasted = colorcontrast_cpu(corrected, 1.5, -2.0, 0.75, 5.0, true);
        let expected = vibrance_opencl_default(contrasted, 0.25);
        for (channel, (actual, expected)) in
            result.pixels().iter().copied().zip(expected).enumerate()
        {
            if channel == 3 {
                assert_eq!(actual.to_bits(), expected.to_bits(), "alpha");
            } else {
                assert!(
                    (actual - expected).abs() <= 0.000_03,
                    "channel {channel}: {actual} != {expected}"
                );
            }
        }
        assert_eq!(result.dispatches(), 3);
    }

    #[tokio::test]
    async fn colorcorrection_dispatch_preserves_extreme_finite_overflow_and_alpha() {
        let Ok(runtime) = GpuRuntime::initialize(crate::GpuRuntimeConfig::default()).await else {
            return;
        };
        if runtime.is_cpu_only() {
            return;
        }
        let input = [f32::MAX, 1.0, -1.0, f32::from_bits(0x3e55_5555)];
        let result = runtime
            .execute_basic_point(BasicPointRequest {
                pixels: &input,
                operations: &[BasicPointOperation::ColorCorrection {
                    saturation: 2.0,
                    a_scale: 1.0,
                    a_base: 0.0,
                    b_scale: -1.0,
                    b_base: 0.0,
                }],
                color_space: BasicPointColorSpace::LabD50,
            })
            .expect("extreme-finite Color Correction point dispatch");
        assert_eq!(result.pixels()[0].to_bits(), input[0].to_bits());
        assert!(result.pixels()[1].is_infinite());
        assert!(result.pixels()[1].is_sign_positive());
        assert!(result.pixels()[2].is_infinite());
        assert!(result.pixels()[2].is_sign_negative());
        assert_eq!(result.pixels()[3].to_bits(), input[3].to_bits());
        assert_eq!(result.dispatches(), 1);
    }

    #[tokio::test]
    async fn colorcontrast_dispatch_matches_native_lab_formula_bound_and_unbound() {
        let Ok(runtime) = GpuRuntime::initialize(crate::GpuRuntimeConfig::default()).await else {
            return;
        };
        if runtime.is_cpu_only() {
            return;
        }
        let corpus = [
            [50.0, 10.0, -20.0, 0.0],
            [80.0, 100.0, -100.0, 0.37],
            [0.0, -128.0, 128.0, 1.0],
            [100.0, 0.0, 0.0, f32::from_bits(1)],
        ];
        let input = corpus.into_iter().flatten().collect::<Vec<_>>();
        for unbound in [false, true] {
            let operation = BasicPointOperation::ColorContrast {
                a_steepness: 2.0,
                a_offset: 3.0,
                b_steepness: 1.5,
                b_offset: -4.0,
                unbound,
            };
            let result = runtime
                .execute_basic_point(BasicPointRequest {
                    pixels: &input,
                    operations: &[operation],
                    color_space: BasicPointColorSpace::LabD50,
                })
                .expect("Color Contrast point dispatch");
            let (actual_pixels, remainder) = result.pixels().as_chunks::<4>();
            assert!(remainder.is_empty());
            for (index, (actual, source)) in actual_pixels.iter().zip(corpus).enumerate() {
                let expected = colorcontrast_cpu(source, 2.0, 3.0, 1.5, -4.0, unbound);
                for channel in 0..3 {
                    assert!(
                        (actual[channel] - expected[channel]).abs() <= 0.00001,
                        "pixel {index} channel {channel}: {} != {}",
                        actual[channel],
                        expected[channel]
                    );
                }
                assert_eq!(
                    actual[3].to_bits(),
                    expected[3].to_bits(),
                    "pixel {index} alpha"
                );
            }
            assert_eq!(result.dispatches(), 1);
        }
    }

    #[tokio::test]
    async fn colorcontrast_dispatch_applies_multiple_instances_in_authored_order() {
        let Ok(runtime) = GpuRuntime::initialize(crate::GpuRuntimeConfig::default()).await else {
            return;
        };
        if runtime.is_cpu_only() {
            return;
        }
        let corpus = [
            [50.0, 10.0, -20.0, 0.37],
            [80.0, 100.0, -100.0, f32::from_bits(1)],
        ];
        let input = corpus.into_iter().flatten().collect::<Vec<_>>();
        let operations = [
            BasicPointOperation::ColorContrast {
                a_steepness: 2.0,
                a_offset: 3.0,
                b_steepness: 1.5,
                b_offset: -4.0,
                unbound: true,
            },
            BasicPointOperation::ColorContrast {
                a_steepness: 0.5,
                a_offset: -2.0,
                b_steepness: 2.0,
                b_offset: 5.0,
                unbound: false,
            },
        ];
        let result = runtime
            .execute_basic_point(BasicPointRequest {
                pixels: &input,
                operations: &operations,
                color_space: BasicPointColorSpace::LabD50,
            })
            .expect("ordered Color Contrast point dispatches");
        let (actual_pixels, remainder) = result.pixels().as_chunks::<4>();
        assert!(remainder.is_empty());
        for (index, (actual, source)) in actual_pixels.iter().zip(corpus).enumerate() {
            let first = colorcontrast_cpu(source, 2.0, 3.0, 1.5, -4.0, true);
            let expected = colorcontrast_cpu(first, 0.5, -2.0, 2.0, 5.0, false);
            for channel in 0..3 {
                assert!(
                    (actual[channel] - expected[channel]).abs() <= 0.00001,
                    "pixel {index} channel {channel}: {} != {}",
                    actual[channel],
                    expected[channel]
                );
            }
            assert_eq!(
                actual[3].to_bits(),
                expected[3].to_bits(),
                "pixel {index} alpha"
            );
        }
        assert_eq!(result.dispatches(), 2);
    }

    #[tokio::test]
    async fn colorcontrast_dispatch_preserves_native_finite_overflow_semantics() {
        let Ok(runtime) = GpuRuntime::initialize(crate::GpuRuntimeConfig::default()).await else {
            return;
        };
        if runtime.is_cpu_only() {
            return;
        }
        let corpus = [
            [50.0, f32::MAX, -f32::MAX, 0.37],
            [80.0, -f32::MAX, f32::MAX, f32::from_bits(1)],
        ];
        let input = corpus.into_iter().flatten().collect::<Vec<_>>();
        assert!(input.iter().all(|component| component.is_finite()));

        for unbound in [false, true] {
            let operation = BasicPointOperation::ColorContrast {
                a_steepness: 2.0,
                a_offset: 0.0,
                b_steepness: 2.0,
                b_offset: 0.0,
                unbound,
            };
            assert!(operation.parameters_are_finite());
            let result = runtime
                .execute_basic_point(BasicPointRequest {
                    pixels: &input,
                    operations: &[operation],
                    color_space: BasicPointColorSpace::LabD50,
                })
                .expect("overflow-derived Color Contrast point dispatch");
            let (actual_pixels, remainder) = result.pixels().as_chunks::<4>();
            assert!(remainder.is_empty());
            for (index, (actual, source)) in actual_pixels.iter().zip(corpus).enumerate() {
                assert_eq!(actual[0].to_bits(), source[0].to_bits(), "pixel {index} L");
                assert_eq!(
                    actual[3].to_bits(),
                    source[3].to_bits(),
                    "pixel {index} alpha"
                );
                for (channel, source_component) in [(1, source[1]), (2, source[2])] {
                    if unbound {
                        assert!(
                            actual[channel].is_infinite(),
                            "pixel {index} channel {channel} must preserve overflow as infinity"
                        );
                        assert_eq!(
                            actual[channel].is_sign_negative(),
                            source_component.is_sign_negative(),
                            "pixel {index} channel {channel} infinity sign"
                        );
                    } else {
                        let expected = if source_component.is_sign_negative() {
                            -128.0_f32
                        } else {
                            128.0_f32
                        };
                        assert_eq!(
                            actual[channel].to_bits(),
                            expected.to_bits(),
                            "pixel {index} channel {channel} exact bounded clamp"
                        );
                    }
                }
            }
            assert_eq!(result.dispatches(), 1);
        }
    }

    #[tokio::test]
    async fn velvia_dispatch_matches_scalar_cpu_corpus_with_clipping_and_exact_alpha() {
        let Ok(runtime) = GpuRuntime::initialize(crate::GpuRuntimeConfig::default()).await else {
            return;
        };
        if runtime.is_cpu_only() {
            return;
        }
        let corpus = [
            [0.1, 0.2, 0.3, 0.0],
            [0.99, 0.90, 0.90, 0.37],
            [2.0, 2.0, 2.0, 1.0],
            [f32::MAX, f32::MAX, f32::MAX, f32::from_bits(0x3eaa_aaab)],
            [-0.25, 0.5, 1.25, f32::from_bits(1)],
        ];
        let input = corpus.into_iter().flatten().collect::<Vec<_>>();
        let operation = BasicPointOperation::Velvia {
            strength: 0.85,
            bias: 0.2,
        };
        let result = runtime
            .execute_basic_point(BasicPointRequest {
                pixels: &input,
                operations: &[operation],
                color_space: BasicPointColorSpace::LinearRgb,
            })
            .expect("Velvia point dispatch");
        let (actual_pixels, remainder) = result.pixels().as_chunks::<4>();
        assert!(remainder.is_empty());
        for (index, (actual, source)) in actual_pixels.iter().zip(corpus).enumerate() {
            let expected = velvia_cpu(source, 0.85, 0.2);
            for channel in 0..3 {
                assert!(
                    (actual[channel] - expected[channel]).abs() <= 0.00001,
                    "pixel {index} channel {channel}: {} != {}",
                    actual[channel],
                    expected[channel]
                );
            }
            assert_eq!(
                actual[3].to_bits(),
                expected[3].to_bits(),
                "pixel {index} alpha"
            );
        }
        assert_eq!(result.dispatches(), 1);
    }

    #[tokio::test]
    async fn zero_strength_velvia_is_bit_exact_identity() {
        let Ok(runtime) = GpuRuntime::initialize(crate::GpuRuntimeConfig::default()).await else {
            return;
        };
        if runtime.is_cpu_only() {
            return;
        }
        let input = [
            -0.25,
            0.5,
            1.25,
            f32::from_bits(1),
            f32::MAX,
            -f32::MAX,
            0.0,
            f32::from_bits(0x3eaa_aaab),
        ];
        let result = runtime
            .execute_basic_point(BasicPointRequest {
                pixels: &input,
                operations: &[BasicPointOperation::Velvia {
                    strength: 0.0,
                    bias: 0.5,
                }],
                color_space: BasicPointColorSpace::LinearRgb,
            })
            .expect("zero-strength Velvia point dispatch");
        assert_eq!(
            result
                .pixels()
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>(),
            input
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>()
        );
        assert_eq!(result.dispatches(), 1);
    }
}
