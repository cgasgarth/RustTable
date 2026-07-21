use std::fmt;

use rusttable_processing::{CompiledOperationGraph, OperationGraphInput, ProcessingOperationKind};
use sha2::{Digest, Sha256};

use crate::{CpuPixelpipeOutputMode, RgbaF32ColorEncoding, RgbaF32Image, SourceRasterIdentity};

/// The stable identity of one prepared CPU pixelpipe snapshot.
///
/// The digest covers the validated source raster, graph provenance and every
/// pixel-affecting prepared operation value. It excludes paths, labels, timing
/// and other mutable presentation state.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct CpuPixelpipeSnapshotIdentity([u8; 32]);

impl CpuPixelpipeSnapshotIdentity {
    #[must_use]
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Debug for CpuPixelpipeSnapshotIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// A fully detached, immutable input to the canonical CPU pixelpipe.
#[derive(Debug, Clone, PartialEq)]
pub struct CpuPixelpipeSnapshot {
    input: RgbaF32Image,
    graph: CompiledOperationGraph,
    output_mode: CpuPixelpipeOutputMode,
    identity: CpuPixelpipeSnapshotIdentity,
}

impl CpuPixelpipeSnapshot {
    /// Detaches validated values into an immutable snapshot.
    #[must_use]
    pub fn new(
        input: RgbaF32Image,
        graph: CompiledOperationGraph,
        output_mode: CpuPixelpipeOutputMode,
    ) -> Self {
        let identity = snapshot_identity(&input, &graph, output_mode);
        Self {
            input,
            graph,
            output_mode,
            identity,
        }
    }

    /// Builds a snapshot while enforcing the initial CPU input boundary.
    ///
    /// # Errors
    ///
    /// Returns [`CpuPixelpipeSnapshotError::UnsupportedInputEncoding`] when
    /// the source raster is not transfer-encoded sRGB.
    pub fn try_new(
        input: RgbaF32Image,
        graph: CompiledOperationGraph,
        output_mode: CpuPixelpipeOutputMode,
    ) -> Result<Self, CpuPixelpipeSnapshotError> {
        if input.descriptor().color_encoding() != RgbaF32ColorEncoding::SrgbD65 {
            return Err(CpuPixelpipeSnapshotError::UnsupportedInputEncoding {
                actual: input.descriptor().color_encoding(),
            });
        }
        Ok(Self::new(input, graph, output_mode))
    }

    #[must_use]
    pub const fn input(&self) -> &RgbaF32Image {
        &self.input
    }

    #[must_use]
    pub const fn graph(&self) -> &CompiledOperationGraph {
        &self.graph
    }

    #[must_use]
    pub const fn output_mode(&self) -> CpuPixelpipeOutputMode {
        self.output_mode
    }

    #[must_use]
    pub const fn source_identity(&self) -> SourceRasterIdentity {
        self.input.source_identity()
    }

    #[must_use]
    pub const fn identity(&self) -> CpuPixelpipeSnapshotIdentity {
        self.identity
    }
}

/// Rejection from immutable CPU snapshot preparation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuPixelpipeSnapshotError {
    UnsupportedInputEncoding { actual: RgbaF32ColorEncoding },
}

impl fmt::Display for CpuPixelpipeSnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedInputEncoding { actual } => write!(
                formatter,
                "CPU pixelpipe snapshot does not accept {actual:?} input"
            ),
        }
    }
}

impl std::error::Error for CpuPixelpipeSnapshotError {}

fn snapshot_identity(
    input: &RgbaF32Image,
    graph: &CompiledOperationGraph,
    output_mode: CpuPixelpipeOutputMode,
) -> CpuPixelpipeSnapshotIdentity {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.cpu-pixelpipe.snapshot.v1");
    hasher.update(input.source_identity().as_bytes());
    hasher.update(input.descriptor().dimensions().width().to_le_bytes());
    hasher.update(input.descriptor().dimensions().height().to_le_bytes());
    hasher.update([encoding_tag(input.descriptor().color_encoding())]);
    hasher.update([mode_tag(output_mode)]);
    write_u128(&mut hasher, graph.source_edit_id().get());
    write_u128(&mut hasher, graph.source_photo_id().get());
    hasher.update(graph.base_photo_revision().get().to_le_bytes());
    hasher.update(graph.revision().get().to_le_bytes());

    for node in graph.nodes() {
        hasher.update((node.index().get() as u64).to_le_bytes());
        hasher.update((node.pipeline_step_index().get() as u64).to_le_bytes());
        match node.input() {
            OperationGraphInput::Source => hasher.update([0]),
            OperationGraphInput::Node(index) => {
                hasher.update([1]);
                hasher.update((index.get() as u64).to_le_bytes());
            }
        }
        write_operation(&mut hasher, node.operation());
    }

    CpuPixelpipeSnapshotIdentity(hasher.finalize().into())
}

fn write_operation(hasher: &mut Sha256, operation: &rusttable_processing::ProcessingOperation) {
    write_u128(hasher, operation.operation_id().get());
    hasher.update([u8::from(operation.is_enabled())]);
    hasher.update(operation.opacity().get().to_bits().to_le_bytes());
    write_operation_kind(hasher, operation.kind());
}

fn write_operation_kind(hasher: &mut Sha256, kind: &ProcessingOperationKind) {
    match kind {
        ProcessingOperationKind::Exposure { .. }
        | ProcessingOperationKind::LinearOffset { .. }
        | ProcessingOperationKind::RgbGain { .. }
        | ProcessingOperationKind::Highlights { .. }
        | ProcessingOperationKind::ColorReconstruction { .. }
        | ProcessingOperationKind::ColorIn { .. }
        | ProcessingOperationKind::Primaries { .. }
        | ProcessingOperationKind::ColorOut { .. } => write_operation_kind_core(hasher, kind),
        _ => write_operation_kind_extended(hasher, kind),
    }
}

fn write_operation_kind_core(hasher: &mut Sha256, kind: &ProcessingOperationKind) {
    match kind {
        ProcessingOperationKind::Exposure { stops } => {
            hasher.update([0]);
            hasher.update(stops.get().to_bits().to_le_bytes());
        }
        ProcessingOperationKind::LinearOffset { value } => {
            hasher.update([1]);
            hasher.update(value.get().to_bits().to_le_bytes());
        }
        ProcessingOperationKind::RgbGain { red, green, blue } => {
            hasher.update([2]);
            hasher.update(red.get().to_bits().to_le_bytes());
            hasher.update(green.get().to_bits().to_le_bytes());
            hasher.update(blue.get().to_bits().to_le_bytes());
        }
        ProcessingOperationKind::Highlights { config } => {
            hasher.update([3]);
            hasher.update(config.method().id().to_le_bytes());
            hasher.update(config.strength().get().to_bits().to_le_bytes());
            hasher.update(config.clip().get().to_bits().to_le_bytes());
            hasher.update(config.noise_level().get().to_bits().to_le_bytes());
            hasher.update(config.iterations().to_le_bytes());
            hasher.update([
                config.scales().id(),
                u8::try_from(config.recovery().id()).expect("recovery mode IDs fit in u8"),
            ]);
            hasher.update(config.candidating().get().to_bits().to_le_bytes());
            hasher.update(config.combine().get().to_bits().to_le_bytes());
            hasher.update(config.solid_color().get().to_bits().to_le_bytes());
        }
        ProcessingOperationKind::ColorReconstruction { config } => {
            hasher.update([4]);
            hasher.update(config.threshold().get().to_bits().to_le_bytes());
            hasher.update(config.spatial().get().to_bits().to_le_bytes());
            hasher.update(config.range().get().to_bits().to_le_bytes());
            hasher.update(config.hue().get().to_bits().to_le_bytes());
            hasher.update([
                u8::try_from(config.precedence().id()).expect("precedence IDs fit in u8")
            ]);
        }
        ProcessingOperationKind::ColorIn { config } => {
            hasher.update([5]);
            let bytes = postcard::to_allocvec(config).expect("colorin config is serializable");
            hasher.update(bytes);
        }
        ProcessingOperationKind::Primaries { config } => {
            hasher.update([6]);
            let bytes = postcard::to_allocvec(config).expect("primaries config is serializable");
            hasher.update(bytes);
        }
        ProcessingOperationKind::ColorOut { config } => {
            hasher.update([7]);
            let bytes = postcard::to_allocvec(config).expect("colorout config is serializable");
            hasher.update(bytes);
        }
        _ => unreachable!("extended operation routed to the extended snapshot writer"),
    }
}

#[allow(clippy::too_many_lines)]
fn write_operation_kind_extended(hasher: &mut Sha256, kind: &ProcessingOperationKind) {
    match kind {
        ProcessingOperationKind::ColorCorrection { config } => {
            hasher.update([8]);
            for value in config.shadow().into_iter().chain(config.highlight()) {
                hasher.update(value.get().to_bits().to_le_bytes());
            }
            hasher.update(config.saturation().get().to_bits().to_le_bytes());
            hasher.update(config.tonal_range().get().to_bits().to_le_bytes());
            hasher.update(config.balance().get().to_bits().to_le_bytes());
            hasher.update([match config.mode() {
                rusttable_processing::operations::colorcorrection::ColorCorrectionMode::TwoColor => 0,
                rusttable_processing::operations::colorcorrection::ColorCorrectionMode::Axis => 1,
            }]);
        }
        ProcessingOperationKind::Temperature { config } => {
            hasher.update([9]);
            hasher.update(config.source().tag().as_bytes());
            hasher.update(config.stage().tag().as_bytes());
            for multiplier in config.multipliers().as_array() {
                hasher.update(multiplier.get().to_bits().to_le_bytes());
            }
            if let Some(pair) = config.temperature_tint() {
                hasher.update(pair.temperature_kelvin().get().to_bits().to_le_bytes());
                hasher.update(pair.tint().get().to_bits().to_le_bytes());
            }
            if let Some(provenance) = config.preset_provenance() {
                hasher.update(provenance.camera_alias().as_bytes());
                hasher.update(provenance.preset_identifier().as_bytes());
                hasher.update(provenance.tuning().to_le_bytes());
                hasher.update(provenance.source_table_revision().to_le_bytes());
            }
        }
        ProcessingOperationKind::Crop { config } => {
            write_crop_operation(hasher, config);
        }
        ProcessingOperationKind::Flip { config } => {
            write_flip_operation(hasher, config);
        }
        ProcessingOperationKind::RotatePixels { config } => {
            let parameters = config.parameters();
            hasher.update([12]);
            hasher.update(parameters.rx.to_le_bytes());
            hasher.update(parameters.ry.to_le_bytes());
            hasher.update(parameters.angle.to_bits().to_le_bytes());
            if let Some(source) = config.opaque_source() {
                hasher.update([1]);
                hasher.update(source);
            } else {
                hasher.update([0]);
            }
        }
        ProcessingOperationKind::ScalePixels { config } => {
            hasher.update([13]);
            hasher.update(config.pixel_aspect_ratio().to_bits().to_le_bytes());
            if let Some(source) = config.opaque_source() {
                hasher.update([1]);
                hasher.update(source);
            } else {
                hasher.update([0]);
            }
        }
        ProcessingOperationKind::FinalScale { config } => {
            hasher.update([14]);
            hasher.update(config.request().identity_bytes());
            hasher.update([
                config.quality().kind().tag(),
                config.quality().kernel().tag(),
            ]);
            hasher.update([u8::from(config.allow_upscale())]);
        }
        ProcessingOperationKind::EnlargeCanvas { config } => {
            hasher.update([15]);
            hasher.update(
                rusttable_processing::operations::enlargecanvas::EnlargeCanvasParametersV1::new(
                    *config,
                )
                .to_bytes(),
            );
        }
        ProcessingOperationKind::Perspective { config } => {
            hasher.update([16]);
            hasher.update(
                rusttable_processing::operations::perspective::PerspectiveParametersV5 {
                    rotation: config.rotation().get(),
                    lensshift_v: config.lensshift_v().get(),
                    lensshift_h: config.lensshift_h().get(),
                    shear: config.shear().get(),
                    focal_length: config.focal_length().get(),
                    crop_factor: config.crop_factor().get(),
                    orthocorr: config.orthocorr().get(),
                    aspect: config.aspect().get(),
                    mode: config.method() as i32,
                    crop_mode: config.crop_mode() as i32,
                    crop_left: config.crop_rectangle()[0],
                    crop_right: config.crop_rectangle()[1],
                    crop_top: config.crop_rectangle()[2],
                    crop_bottom: config.crop_rectangle()[3],
                    last_drawn_lines: config
                        .drawn_lines()
                        .iter()
                        .map(|line| line.map(rusttable_processing::FiniteF32::get))
                        .collect(),
                    last_quad: config.quad(),
                }
                .to_bytes(),
            );
            if let Some(source) = config.opaque_source() {
                hasher.update([1]);
                hasher.update(source);
            } else {
                hasher.update([0]);
            }
        }
        ProcessingOperationKind::LensCorrection { config } => {
            hasher.update([17]);
            hasher.update(config.canonical_identity_bytes());
        }
        ProcessingOperationKind::Bloom { config } => {
            hasher.update([18]);
            hasher.update(
                rusttable_processing::operations::bloom::BloomParametersV1::new(
                    config.size(),
                    config.threshold(),
                    config.strength(),
                )
                .to_bytes(),
            );
        }
        ProcessingOperationKind::Soften { config } => {
            hasher.update([19]);
            hasher.update(
                rusttable_processing::operations::soften::SoftenParametersV1::new(
                    config.size(),
                    config.saturation(),
                    config.brightness(),
                    config.amount(),
                )
                .to_bytes(),
            );
        }
        _ => unreachable!("core operation routed to the core snapshot writer"),
    }
}

fn write_crop_operation(
    hasher: &mut Sha256,
    config: &rusttable_processing::operations::crop::CropConfig,
) {
    hasher.update([10]);
    hasher.update(config.cx().get().to_bits().to_le_bytes());
    hasher.update(config.cy().get().to_bits().to_le_bytes());
    hasher.update(config.cw().get().to_bits().to_le_bytes());
    hasher.update(config.ch().get().to_bits().to_le_bytes());
    hasher.update(config.ratio_n().to_le_bytes());
    hasher.update(config.ratio_d().to_le_bytes());
}

fn write_flip_operation(
    hasher: &mut Sha256,
    config: &rusttable_processing::operations::flip::FlipConfig,
) {
    hasher.update([11]);
    hasher.update([match config.mode() {
        rusttable_processing::operations::flip::FlipMode::Automatic => 0,
        rusttable_processing::operations::flip::FlipMode::Explicit => 1,
    }]);
    hasher.update([config.orientation().bits()]);
    if let Some(source) = config.opaque_source() {
        hasher.update([1]);
        hasher.update(source);
    } else {
        hasher.update([0]);
    }
}

fn write_u128(hasher: &mut Sha256, value: u128) {
    hasher.update(value.to_le_bytes());
}

const fn encoding_tag(encoding: RgbaF32ColorEncoding) -> u8 {
    match encoding {
        RgbaF32ColorEncoding::SrgbD65 => 0,
        RgbaF32ColorEncoding::LinearSrgbD65 => 1,
    }
}

const fn mode_tag(mode: CpuPixelpipeOutputMode) -> u8 {
    match mode {
        CpuPixelpipeOutputMode::Preview => 0,
        CpuPixelpipeOutputMode::FullExport => 1,
    }
}
