use std::fmt;

use rusttable_masks::{MaskGraph, RasterMaskStore};
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
    mask_graph: Option<MaskGraph>,
    mask_store: Option<RasterMaskStore>,
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
        let mut snapshot = Self {
            input,
            graph,
            output_mode,
            mask_graph: None,
            mask_store: None,
            identity: CpuPixelpipeSnapshotIdentity([0; 32]),
        };
        snapshot.refresh_identity();
        snapshot
    }

    /// Builds a snapshot while enforcing the initial CPU input boundary.
    ///
    /// # Errors
    ///
    /// Returns [`CpuPixelpipeSnapshotError::UnsupportedInputEncoding`] when
    /// the source raster is neither transfer-encoded sRGB nor an explicit Lab
    /// compatibility raster.
    pub fn try_new(
        input: RgbaF32Image,
        graph: CompiledOperationGraph,
        output_mode: CpuPixelpipeOutputMode,
    ) -> Result<Self, CpuPixelpipeSnapshotError> {
        if !matches!(
            input.descriptor().color_encoding(),
            RgbaF32ColorEncoding::SrgbD65
                | RgbaF32ColorEncoding::LinearSrgbD65
                | RgbaF32ColorEncoding::DisplayP3D65
                | RgbaF32ColorEncoding::LinearDisplayP3D65
                | RgbaF32ColorEncoding::External(_)
                | RgbaF32ColorEncoding::LabD50
        ) {
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

    /// Attaches the immutable mask graph used by CPU evaluation.
    #[must_use]
    pub fn with_mask_graph(mut self, graph: MaskGraph) -> Self {
        self.mask_graph = Some(graph);
        self.refresh_identity();
        self
    }

    /// Attaches the bounded publication store used by generated mask nodes.
    #[must_use]
    pub fn with_mask_store(mut self, store: RasterMaskStore) -> Self {
        self.mask_store = Some(store);
        self.refresh_identity();
        self
    }

    #[must_use]
    pub const fn mask_graph(&self) -> Option<&MaskGraph> {
        self.mask_graph.as_ref()
    }

    #[must_use]
    pub const fn mask_store(&self) -> Option<&RasterMaskStore> {
        self.mask_store.as_ref()
    }

    #[must_use]
    pub const fn source_identity(&self) -> SourceRasterIdentity {
        self.input.source_identity()
    }

    #[must_use]
    pub const fn identity(&self) -> CpuPixelpipeSnapshotIdentity {
        self.identity
    }

    fn refresh_identity(&mut self) {
        self.identity = snapshot_identity(
            &self.input,
            &self.graph,
            self.output_mode,
            self.mask_graph.as_ref(),
            self.mask_store.as_ref(),
        );
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
    mask_graph: Option<&MaskGraph>,
    mask_store: Option<&RasterMaskStore>,
) -> CpuPixelpipeSnapshotIdentity {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.cpu-pixelpipe.snapshot.v2");
    hasher.update(input.source_identity().as_bytes());
    hasher.update(input.descriptor().dimensions().width().to_le_bytes());
    hasher.update(input.descriptor().dimensions().height().to_le_bytes());
    hasher.update([encoding_tag(input.descriptor().color_encoding())]);
    write_source_color(&mut hasher, input.descriptor().source_color());
    hasher.update([input.descriptor().source_orientation() as u8]);
    hasher.update([mode_tag(output_mode)]);
    if let Some(mask_graph) = mask_graph {
        hasher.update([1]);
        hasher.update(mask_graph.identity());
    } else {
        hasher.update([0]);
    }
    if let Some(mask_store) = mask_store {
        hasher.update([1]);
        hasher.update(mask_store.identity());
    } else {
        hasher.update([0]);
    }
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
        ProcessingOperationKind::BasicAdj { .. }
        | ProcessingOperationKind::Exposure { .. }
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
        ProcessingOperationKind::BasicAdj { config } => {
            hasher.update([21]);
            for value in [
                config.black_point(),
                config.exposure(),
                config.hlcompr(),
                config.hlcomprthresh(),
                config.contrast(),
                config.middle_grey(),
                config.brightness(),
                config.saturation(),
                config.vibrance(),
                config.clip(),
            ] {
                hasher.update(value.to_bits().to_le_bytes());
            }
            hasher.update(config.preserve_colors().id().to_le_bytes());
            hasher.update([config.auto_controls().bits()]);
        }
        ProcessingOperationKind::Exposure { stops, black } => {
            hasher.update([0]);
            hasher.update(stops.get().to_bits().to_le_bytes());
            hasher.update(black.get().to_bits().to_le_bytes());
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
        ProcessingOperationKind::Clipping { config } => {
            hasher.update([21]);
            hasher.update(config.parameters().to_bytes());
            if let Some(source) = config.opaque_source() {
                hasher.update([1]);
                hasher.update(source);
            } else {
                hasher.update([0]);
            }
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
        ProcessingOperationKind::Grain { config } => {
            hasher.update([20]);
            hasher.update(config.parameters().to_bytes());
            hasher.update(config.seed().to_le_bytes());
        }
        ProcessingOperationKind::Censorize { config } => {
            hasher.update([22]);
            hasher.update(config.parameters().to_bytes());
        }
        ProcessingOperationKind::Defringe { config } => {
            hasher.update([23]);
            hasher.update(config.parameters().to_bytes());
        }
        ProcessingOperationKind::Clahe { config } => {
            hasher.update([24]);
            hasher.update(config.parameters().to_bytes());
        }
        ProcessingOperationKind::Shadhi { config } => {
            hasher.update([25]);
            hasher.update(
                rusttable_processing::operations::shadhi::ShadhiParametersV5 {
                    order: config.order(),
                    radius: config.radius(),
                    shadows: config.shadows(),
                    whitepoint: config.whitepoint(),
                    highlights: config.highlights(),
                    reserved2: config.reserved2(),
                    compress: config.compress(),
                    shadows_ccorrect: config.shadows_ccorrect(),
                    highlights_ccorrect: config.highlights_ccorrect(),
                    flags: config.flags(),
                    low_approximation: config.low_approximation(),
                    shadhi_algo: config.shadhi_algo().id(),
                }
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
        RgbaF32ColorEncoding::DisplayP3D65 => 2,
        RgbaF32ColorEncoding::LinearDisplayP3D65 => 3,
        RgbaF32ColorEncoding::External(_) => 4,
        RgbaF32ColorEncoding::LabD50 => 5,
        RgbaF32ColorEncoding::Rec2020D65 => 6,
        RgbaF32ColorEncoding::LinearRec2020D65 => 7,
        RgbaF32ColorEncoding::AcesCgD60 => 8,
    }
}

fn write_source_color(hasher: &mut Sha256, source: Option<rusttable_image::SourceColor>) {
    let Some(source) = source else {
        hasher.update([0]);
        return;
    };
    hasher.update([1]);
    hasher.update(postcard::to_allocvec(&source.encoding()).expect("color encoding serializes"));
    if let Some((primaries, transfer)) = source.matrix() {
        hasher.update([0]);
        hasher.update(postcard::to_allocvec(&transfer).expect("transfer serializes"));
        for pair in [primaries.red(), primaries.green(), primaries.blue()] {
            hasher.update(pair.0.get().to_bits().to_le_bytes());
            hasher.update(pair.1.get().to_bits().to_le_bytes());
        }
        let (white_x, white_y) = primaries.white().xy();
        hasher.update(white_x.to_bits().to_le_bytes());
        hasher.update(white_y.to_bits().to_le_bytes());
    } else {
        hasher.update([1]);
    }
    if let Some(profile) = source.profile() {
        hasher.update([1]);
        hasher.update(postcard::to_allocvec(&profile).expect("profile identity serializes"));
    } else {
        hasher.update([0]);
    }
    hasher.update([match source.evidence() {
        rusttable_image::SourceColorEvidence::DeclaredEncoding => 0,
        rusttable_image::SourceColorEvidence::EmbeddedIcc => 1,
        rusttable_image::SourceColorEvidence::EmbeddedChromaticities => 2,
        rusttable_image::SourceColorEvidence::EmbeddedContainerMetadata => 3,
        rusttable_image::SourceColorEvidence::Fallback(
            rusttable_image::SourceColorFallback::EncodedSrgb,
        ) => 4,
        rusttable_image::SourceColorEvidence::Fallback(
            rusttable_image::SourceColorFallback::LinearRec709,
        ) => 5,
    }]);
}

const fn mode_tag(mode: CpuPixelpipeOutputMode) -> u8 {
    match mode {
        CpuPixelpipeOutputMode::Preview => 0,
        CpuPixelpipeOutputMode::FullExport => 1,
    }
}
