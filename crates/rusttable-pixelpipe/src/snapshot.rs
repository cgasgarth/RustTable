use std::fmt;

use rusttable_masks::{MaskGraph, RasterMaskStore};
use rusttable_processing::{CompiledOperationGraph, OperationGraphInput, ProcessingOperationKind};
use sha2::{Digest, Sha256};

const BASICADJ_SNAPSHOT_KIND_TAG: u8 = 21;
const CLIPPING_SNAPSHOT_KIND_TAG: u8 = 29;
const COLORZONES_SNAPSHOT_KIND_TAG: u8 = 30;
const SHARPEN_SNAPSHOT_KIND_TAG: u8 = 31;
const CHANNEL_MIXER_SNAPSHOT_KIND_TAG: u8 = 32;
const AGX_SNAPSHOT_KIND_TAG: u8 = 33;
const LEVELS_SNAPSHOT_KIND_TAG: u8 = 34;
const RGBLEVELS_SNAPSHOT_KIND_TAG: u8 = 35;
const COLORTRANSFER_SNAPSHOT_KIND_TAG: u8 = 36;
const COLORMAPPING_SNAPSHOT_KIND_TAG: u8 = 37;
const SNAPSHOT_KIND_TAGS: [u8; 38] = [
    0,
    1,
    2,
    3,
    4,
    5,
    6,
    7,
    8,
    9,
    10,
    11,
    12,
    13,
    14,
    15,
    16,
    17,
    18,
    19,
    20,
    BASICADJ_SNAPSHOT_KIND_TAG,
    22,
    23,
    24,
    25,
    26,
    27,
    28,
    CLIPPING_SNAPSHOT_KIND_TAG,
    COLORZONES_SNAPSHOT_KIND_TAG,
    SHARPEN_SNAPSHOT_KIND_TAG,
    CHANNEL_MIXER_SNAPSHOT_KIND_TAG,
    AGX_SNAPSHOT_KIND_TAG,
    LEVELS_SNAPSHOT_KIND_TAG,
    RGBLEVELS_SNAPSHOT_KIND_TAG,
    COLORTRANSFER_SNAPSHOT_KIND_TAG,
    COLORMAPPING_SNAPSHOT_KIND_TAG,
];
const _: () = assert!(snapshot_kind_tags_are_unique(&SNAPSHOT_KIND_TAGS));

const fn snapshot_kind_tags_are_unique(tags: &[u8]) -> bool {
    let mut index = 0;
    while index < tags.len() {
        let mut candidate = index + 1;
        while candidate < tags.len() {
            if tags[index] == tags[candidate] {
                return false;
            }
            candidate += 1;
        }
        index += 1;
    }
    true
}

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

/// Immutable scale values used by native ROI-dependent operation planning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CpuPixelpipeScaleContext {
    roi_scale: rusttable_processing::FiniteF32,
    piece_iscale: rusttable_processing::FiniteF32,
}

impl CpuPixelpipeScaleContext {
    /// Creates a finite, positive equivalent of Darktable's
    /// `roi_in->scale` and `piece->iscale` pair.
    ///
    /// # Errors
    ///
    /// Returns [`CpuPixelpipeSnapshotError::InvalidScaleContext`] when either
    /// value is non-finite, zero, or negative.
    pub fn new(roi_scale: f32, piece_iscale: f32) -> Result<Self, CpuPixelpipeSnapshotError> {
        let roi_scale = rusttable_processing::FiniteF32::new(roi_scale)
            .map_err(|_| CpuPixelpipeSnapshotError::InvalidScaleContext)?;
        let piece_iscale = rusttable_processing::FiniteF32::new(piece_iscale)
            .map_err(|_| CpuPixelpipeSnapshotError::InvalidScaleContext)?;
        if roi_scale.get() <= 0.0 || piece_iscale.get() <= 0.0 {
            return Err(CpuPixelpipeSnapshotError::InvalidScaleContext);
        }
        Ok(Self {
            roi_scale,
            piece_iscale,
        })
    }

    #[must_use]
    pub const fn roi_scale(self) -> f32 {
        self.roi_scale.get()
    }

    #[must_use]
    pub const fn piece_iscale(self) -> f32 {
        self.piece_iscale.get()
    }
}

impl Default for CpuPixelpipeScaleContext {
    fn default() -> Self {
        Self::new(1.0, 1.0).expect("unit scale context is valid")
    }
}

/// A fully detached, immutable input to the canonical CPU pixelpipe.
#[derive(Debug, Clone, PartialEq)]
pub struct CpuPixelpipeSnapshot {
    input: RgbaF32Image,
    graph: CompiledOperationGraph,
    output_mode: CpuPixelpipeOutputMode,
    scale_context: CpuPixelpipeScaleContext,
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
            scale_context: CpuPixelpipeScaleContext::default(),
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

    #[must_use]
    pub const fn scale_context(&self) -> CpuPixelpipeScaleContext {
        self.scale_context
    }

    /// Attaches immutable native ROI scale context and refreshes snapshot identity.
    #[must_use]
    pub fn with_scale_context(mut self, context: CpuPixelpipeScaleContext) -> Self {
        self.scale_context = context;
        self.refresh_identity();
        self
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
            self.scale_context,
            self.mask_graph.as_ref(),
            self.mask_store.as_ref(),
        );
    }
}

/// Rejection from immutable CPU snapshot preparation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuPixelpipeSnapshotError {
    UnsupportedInputEncoding { actual: RgbaF32ColorEncoding },
    InvalidScaleContext,
}

impl fmt::Display for CpuPixelpipeSnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedInputEncoding { actual } => write!(
                formatter,
                "CPU pixelpipe snapshot does not accept {actual:?} input"
            ),
            Self::InvalidScaleContext => formatter
                .write_str("CPU pixelpipe ROI scale and piece iscale must be finite and positive"),
        }
    }
}

impl std::error::Error for CpuPixelpipeSnapshotError {}

fn snapshot_identity(
    input: &RgbaF32Image,
    graph: &CompiledOperationGraph,
    output_mode: CpuPixelpipeOutputMode,
    scale_context: CpuPixelpipeScaleContext,
    mask_graph: Option<&MaskGraph>,
    mask_store: Option<&RasterMaskStore>,
) -> CpuPixelpipeSnapshotIdentity {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.cpu-pixelpipe.snapshot.v3");
    hasher.update(input.source_identity().as_bytes());
    hasher.update(input.descriptor().dimensions().width().to_le_bytes());
    hasher.update(input.descriptor().dimensions().height().to_le_bytes());
    hasher.update([encoding_tag(input.descriptor().color_encoding())]);
    write_source_color(&mut hasher, input.descriptor().source_color());
    hasher.update([input.descriptor().source_orientation() as u8]);
    hasher.update([mode_tag(output_mode)]);
    hasher.update(scale_context.roi_scale().to_bits().to_le_bytes());
    hasher.update(scale_context.piece_iscale().to_bits().to_le_bytes());
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
            hasher.update([BASICADJ_SNAPSHOT_KIND_TAG]);
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
        ProcessingOperationKind::Agx { config } => {
            hasher.update([AGX_SNAPSHOT_KIND_TAG]);
            hasher.update(config.parameters().to_bytes());
        }
        ProcessingOperationKind::Levels { config } => {
            hasher.update([LEVELS_SNAPSHOT_KIND_TAG]);
            hasher.update(config.parameters().to_bytes());
        }
        ProcessingOperationKind::RgbLevels { config } => {
            hasher.update([RGBLEVELS_SNAPSHOT_KIND_TAG]);
            hasher.update(config.parameters().to_bytes());
        }
        ProcessingOperationKind::ColorTransfer { parameters } => {
            hasher.update([COLORTRANSFER_SNAPSHOT_KIND_TAG]);
            hasher.update(parameters.to_bytes());
        }
        ProcessingOperationKind::ColorMapping { config } => {
            hasher.update([COLORMAPPING_SNAPSHOT_KIND_TAG]);
            hasher.update(config.parameters().to_bytes());
        }
        ProcessingOperationKind::ColorCorrection { config } => {
            hasher.update([8]);
            for value in config.committed_coefficients().as_array() {
                hasher.update(value.to_bits().to_le_bytes());
            }
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
            hasher.update([CLIPPING_SNAPSHOT_KIND_TAG]);
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
        ProcessingOperationKind::Velvia { config } => {
            hasher.update([26]);
            hasher.update(config.strength().to_bits().to_le_bytes());
            hasher.update(config.bias().to_bits().to_le_bytes());
        }
        ProcessingOperationKind::ChannelMixer { config } => {
            hasher.update([CHANNEL_MIXER_SNAPSHOT_KIND_TAG]);
            for value in config
                .red()
                .into_iter()
                .chain(config.green())
                .chain(config.blue())
            {
                hasher.update(value.to_bits().to_le_bytes());
            }
            hasher.update((config.algorithm_version() as i32).to_le_bytes());
        }
        ProcessingOperationKind::ColorContrast { config } => {
            hasher.update([27]);
            hasher.update(config.a_steepness().to_bits().to_le_bytes());
            hasher.update(config.a_offset().to_bits().to_le_bytes());
            hasher.update(config.b_steepness().to_bits().to_le_bytes());
            hasher.update(config.b_offset().to_bits().to_le_bytes());
            hasher.update(config.unbound().to_le_bytes());
        }
        ProcessingOperationKind::Vibrance { config } => {
            hasher.update([28]);
            hasher.update(config.amount().to_bits().to_le_bytes());
        }
        ProcessingOperationKind::ColorZones { plan } => {
            let config = plan.config();
            hasher.update([COLORZONES_SNAPSHOT_KIND_TAG]);
            hasher.update(config.channel().raw().to_le_bytes());
            hasher.update(config.strength().to_bits().to_le_bytes());
            hasher.update(config.mode().raw().to_le_bytes());
            hasher.update(config.splines_version().raw().to_le_bytes());
            for curve in config.curves() {
                hasher.update(curve.curve_type().raw().to_le_bytes());
                hasher.update(
                    u64::try_from(curve.node_count())
                        .expect("Color Zones node count fits u64")
                        .to_le_bytes(),
                );
                for point in curve.points() {
                    hasher.update(point.x().to_bits().to_le_bytes());
                    hasher.update(point.y().to_bits().to_le_bytes());
                }
            }
        }
        ProcessingOperationKind::Sharpen { config } => {
            hasher.update([SHARPEN_SNAPSHOT_KIND_TAG]);
            hasher.update(config.parameters().to_bytes());
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
