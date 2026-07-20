use rusttable_gpu::{BasicPointError, BasicPointOperation, BasicPointRequest, GpuRuntime};
use rusttable_processing::{
    FiniteF32, LinearRgb, SourceRgb, SourceRgbImage, SrgbChannel, WorkingRgbImage,
    encode_linear_srgb, to_linear_srgb,
};

use crate::{
    CpuPixelpipeError, CpuPixelpipeExecutor, CpuPixelpipeOutputMode, CpuPixelpipeSnapshot,
    RgbaF32ColorEncoding, RgbaF32Descriptor, RgbaF32Image, RgbaF32ImageError, RgbaF32Pixel,
};

/// The backend that published one pixelpipe result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelpipeBackend {
    CpuCanonical,
    WgpuBasic,
}

/// Bounded provenance for one service execution attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PixelpipeExecutionReceipt {
    snapshot_identity: crate::CpuPixelpipeSnapshotIdentity,
    backend: PixelpipeBackend,
    gpu_fallback: Option<BasicPointError>,
    dispatches: u32,
}

impl PixelpipeExecutionReceipt {
    #[must_use]
    pub const fn snapshot_identity(&self) -> crate::CpuPixelpipeSnapshotIdentity {
        self.snapshot_identity
    }

    #[must_use]
    pub const fn backend(&self) -> PixelpipeBackend {
        self.backend
    }

    #[must_use]
    pub const fn gpu_fallback(&self) -> Option<&BasicPointError> {
        self.gpu_fallback.as_ref()
    }

    #[must_use]
    pub const fn dispatches(&self) -> u32 {
        self.dispatches
    }
}

/// An image and the backend receipt that authorized its publication.
#[derive(Debug, Clone, PartialEq)]
pub struct PixelpipeExecutionResult {
    image: RgbaF32Image,
    receipt: PixelpipeExecutionReceipt,
}

impl PixelpipeExecutionResult {
    #[must_use]
    pub const fn image(&self) -> &RgbaF32Image {
        &self.image
    }

    #[must_use]
    pub const fn receipt(&self) -> &PixelpipeExecutionReceipt {
        &self.receipt
    }
}

/// Application-facing basic pixelpipe coordinator.
///
/// GPU eligibility is derived from the immutable snapshot. The coordinator
/// never skips an enabled unsupported node and always retains the canonical
/// CPU executor as the publication path when GPU preparation or execution
/// fails.
#[derive(Debug)]
pub struct PixelpipeExecutionService {
    cpu: CpuPixelpipeExecutor,
    gpu: Option<GpuRuntime>,
}

impl PixelpipeExecutionService {
    #[must_use]
    pub const fn cpu_only() -> Self {
        Self {
            cpu: CpuPixelpipeExecutor,
            gpu: None,
        }
    }

    #[must_use]
    pub fn with_gpu(gpu: GpuRuntime) -> Self {
        Self {
            cpu: CpuPixelpipeExecutor,
            gpu: Some(gpu),
        }
    }

    /// Executes the snapshot, selecting WGPU only for the currently qualified
    /// basic point range and otherwise publishing the canonical CPU result.
    ///
    /// # Errors
    ///
    /// Returns the canonical pixelpipe error when CPU publication fails.
    pub fn execute(
        &self,
        snapshot: &CpuPixelpipeSnapshot,
    ) -> Result<PixelpipeExecutionResult, CpuPixelpipeError> {
        let Some(operations) = basic_operations(snapshot) else {
            return self.cpu_result(snapshot, None);
        };
        let Some(gpu) = self.gpu.as_ref() else {
            return self.cpu_result(snapshot, None);
        };
        if !gpu.health_check() {
            return self.cpu_result(snapshot, Some(BasicPointError::Unhealthy));
        }

        match execute_gpu(gpu, snapshot, &operations) {
            Ok((image, dispatches)) => Ok(PixelpipeExecutionResult {
                image,
                receipt: PixelpipeExecutionReceipt {
                    snapshot_identity: snapshot.identity(),
                    backend: PixelpipeBackend::WgpuBasic,
                    gpu_fallback: None,
                    dispatches,
                },
            }),
            Err(error) => self.cpu_result(snapshot, Some(error)),
        }
    }

    fn cpu_result(
        &self,
        snapshot: &CpuPixelpipeSnapshot,
        fallback: Option<BasicPointError>,
    ) -> Result<PixelpipeExecutionResult, CpuPixelpipeError> {
        let cpu_result = self.cpu.execute(snapshot)?;
        let image = cpu_result.image().clone();
        Ok(PixelpipeExecutionResult {
            image,
            receipt: PixelpipeExecutionReceipt {
                snapshot_identity: snapshot.identity(),
                backend: PixelpipeBackend::CpuCanonical,
                gpu_fallback: fallback,
                dispatches: 0,
            },
        })
    }
}

fn basic_operations(snapshot: &CpuPixelpipeSnapshot) -> Option<Vec<BasicPointOperation>> {
    let mut operations = Vec::new();
    for node in snapshot.graph().nodes() {
        let operation = node.operation();
        if !operation.is_enabled() {
            continue;
        }
        if operation.opacity().get().to_bits() != 1.0_f32.to_bits() {
            return None;
        }
        let gpu_operation = match operation.kind() {
            rusttable_processing::ProcessingOperationKind::Exposure { stops } => {
                BasicPointOperation::Exposure { stops: stops.get() }
            }
            rusttable_processing::ProcessingOperationKind::LinearOffset { value } => {
                BasicPointOperation::LinearOffset { value: value.get() }
            }
            rusttable_processing::ProcessingOperationKind::RgbGain { red, green, blue } => {
                BasicPointOperation::RgbGain {
                    red: red.get(),
                    green: green.get(),
                    blue: blue.get(),
                }
            }
            _ => return None,
        };
        operations.push(gpu_operation);
    }
    Some(operations)
}

fn execute_gpu(
    gpu: &GpuRuntime,
    snapshot: &CpuPixelpipeSnapshot,
    operations: &[BasicPointOperation],
) -> Result<(RgbaF32Image, u32), BasicPointError> {
    let input = snapshot.input();
    let dimensions = input.descriptor().dimensions();
    let source_pixels = input
        .pixels()
        .iter()
        .copied()
        .map(|pixel| {
            Ok(SourceRgb::new(
                SrgbChannel::new(pixel.red())
                    .map_err(|_| BasicPointError::NonFiniteInput { component: 0 })?,
                SrgbChannel::new(pixel.green())
                    .map_err(|_| BasicPointError::NonFiniteInput { component: 1 })?,
                SrgbChannel::new(pixel.blue())
                    .map_err(|_| BasicPointError::NonFiniteInput { component: 2 })?,
            ))
        })
        .collect::<Result<Vec<_>, BasicPointError>>()?;
    let source = SourceRgbImage::new(dimensions, source_pixels)
        .map_err(|_| BasicPointError::InvalidPixelPacking)?;
    let linear = to_linear_srgb(&source);
    let mut packed = Vec::with_capacity(input.pixels().len() * 4);
    for (working, source) in linear.pixels().zip(input.pixels()) {
        packed.extend([
            working.red().get(),
            working.green().get(),
            working.blue().get(),
            source.alpha(),
        ]);
    }
    let result = gpu.execute_basic_point(BasicPointRequest {
        pixels: &packed,
        operations,
    })?;
    let (packed_pixels, remainder) = result.pixels().as_chunks::<4>();
    debug_assert!(remainder.is_empty(), "GPU output must contain RGBA pixels");
    let mut working_pixels = Vec::with_capacity(input.pixels().len());
    for (index, pixel) in packed_pixels.iter().enumerate() {
        working_pixels.push(LinearRgb::new(
            FiniteF32::new(pixel[0]).map_err(|_| BasicPointError::NonFiniteInput {
                component: index * 4,
            })?,
            FiniteF32::new(pixel[1]).map_err(|_| BasicPointError::NonFiniteInput {
                component: index * 4 + 1,
            })?,
            FiniteF32::new(pixel[2]).map_err(|_| BasicPointError::NonFiniteInput {
                component: index * 4 + 2,
            })?,
        ));
    }
    let working = WorkingRgbImage::new(dimensions, working_pixels)
        .map_err(|_| BasicPointError::InvalidPixelPacking)?;
    let output_pixels = match snapshot.output_mode() {
        CpuPixelpipeOutputMode::FullExport => working
            .pixels()
            .zip(input.pixels())
            .map(|(pixel, source)| {
                RgbaF32Pixel::new(
                    pixel.red().get(),
                    pixel.green().get(),
                    pixel.blue().get(),
                    source.alpha(),
                )
            })
            .collect(),
        CpuPixelpipeOutputMode::Preview => encode_linear_srgb(&working)
            .image()
            .pixels()
            .zip(input.pixels())
            .map(|(pixel, source)| {
                RgbaF32Pixel::new(
                    pixel.red().get(),
                    pixel.green().get(),
                    pixel.blue().get(),
                    source.alpha(),
                )
            })
            .collect(),
    };
    let encoding = match snapshot.output_mode() {
        CpuPixelpipeOutputMode::Preview => RgbaF32ColorEncoding::SrgbD65,
        CpuPixelpipeOutputMode::FullExport => RgbaF32ColorEncoding::LinearSrgbD65,
    };
    let descriptor = RgbaF32Descriptor::new(dimensions, encoding);
    let image = RgbaF32Image::new(descriptor, output_pixels).map_err(|source| match source {
        RgbaF32ImageError::NonFiniteComponent { .. }
        | RgbaF32ImageError::ComponentOutsideUnitInterval { .. }
        | RgbaF32ImageError::PixelCountMismatch { .. }
        | RgbaF32ImageError::SourceIdentityMismatch { .. } => {
            BasicPointError::Readback("GPU output failed the typed image boundary".to_owned())
        }
    })?;
    Ok((image, result.dispatches()))
}
