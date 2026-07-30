use std::fmt;
use std::path::{Path, PathBuf};

use rusttable_color::{ColorEncoding, Matrix3, TransformPlan};
use rusttable_image::{ImageDimensions, Orientation, RawMosaic, Roi};
use sha2::{Digest, Sha256};

pub const RAW_LINEAR_WORKFLOW_VERSION: u16 = 1;
pub const RAW_LINEAR_PLAN_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RawLinearPreparationMode {
    AiRawLinearPreparation,
}

impl RawLinearPreparationMode {
    #[must_use]
    pub const fn tag(self) -> &'static str {
        "ai_raw_linear_preparation"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Strength(u16);

impl Strength {
    pub fn new(value: f32) -> Result<Self, StrengthError> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(StrengthError::OutOfRange);
        }
        Ok(Self((value * 10_000.0).round() as u16))
    }

    #[must_use]
    pub const fn value(self) -> f32 {
        self.0 as f32 / 10_000.0
    }

    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrengthError {
    OutOfRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RawLinearSourceKind {
    XTrans,
    AlreadyLinearRgb,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawLinearCalibration {
    identity: [u8; 32],
    camera_to_rec2020: Matrix3,
}

impl RawLinearCalibration {
    pub fn new(identity: [u8; 32], camera_to_rec2020: Matrix3) -> Result<Self, CalibrationError> {
        if identity == [0; 32] || camera_to_rec2020.inverse().is_err() {
            return Err(CalibrationError::Invalid);
        }
        Ok(Self {
            identity,
            camera_to_rec2020,
        })
    }

    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }

    #[must_use]
    pub const fn camera_to_rec2020(&self) -> Matrix3 {
        self.camera_to_rec2020
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalibrationError {
    Invalid,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LinearRawRgbU16 {
    dimensions: ImageDimensions,
    samples: Vec<u16>,
    black_level: u16,
    white_level: u16,
    color: ColorEncoding,
    orientation: Orientation,
    camera_identity: [u8; 32],
    source_identity: [u8; 32],
}

impl LinearRawRgbU16 {
    pub fn new(
        dimensions: ImageDimensions,
        samples: Vec<u16>,
        black_level: u16,
        white_level: u16,
        color: ColorEncoding,
        orientation: Orientation,
        camera_identity: [u8; 32],
    ) -> Result<Self, LinearRawRgbU16Error> {
        if !color.is_linear() || camera_identity == [0; 32] {
            return Err(LinearRawRgbU16Error::InvalidInterpretation);
        }
        let pixels = usize::try_from(
            dimensions
                .pixel_count()
                .map_err(|_| LinearRawRgbU16Error::ArithmeticOverflow)?,
        )
        .map_err(|_| LinearRawRgbU16Error::ArithmeticOverflow)?;
        let expected = pixels
            .checked_mul(3)
            .ok_or(LinearRawRgbU16Error::ArithmeticOverflow)?;
        if samples.len() != expected {
            return Err(LinearRawRgbU16Error::SampleLength {
                expected,
                actual: samples.len(),
            });
        }
        if white_level <= black_level {
            return Err(LinearRawRgbU16Error::InvalidLevels);
        }
        if let Some((index, value)) = samples
            .iter()
            .copied()
            .enumerate()
            .find(|(_, value)| *value > white_level)
        {
            return Err(LinearRawRgbU16Error::SampleOutsideWhite { index, value });
        }
        let source_identity = source_identity(
            RawLinearSourceKind::AlreadyLinearRgb,
            dimensions,
            &samples,
            camera_identity,
            color,
        );
        Ok(Self {
            dimensions,
            samples,
            black_level,
            white_level,
            color,
            orientation,
            camera_identity,
            source_identity,
        })
    }

    #[must_use]
    pub const fn dimensions(&self) -> ImageDimensions {
        self.dimensions
    }
    #[must_use]
    pub fn samples(&self) -> &[u16] {
        &self.samples
    }
    #[must_use]
    pub const fn black_level(&self) -> u16 {
        self.black_level
    }
    #[must_use]
    pub const fn white_level(&self) -> u16 {
        self.white_level
    }
    #[must_use]
    pub const fn color(&self) -> ColorEncoding {
        self.color
    }
    #[must_use]
    pub const fn orientation(&self) -> Orientation {
        self.orientation
    }
    #[must_use]
    pub const fn camera_identity(&self) -> [u8; 32] {
        self.camera_identity
    }
    #[must_use]
    pub const fn source_identity(&self) -> [u8; 32] {
        self.source_identity
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinearRawRgbU16Error {
    ArithmeticOverflow,
    InvalidInterpretation,
    InvalidLevels,
    SampleLength { expected: usize, actual: usize },
    SampleOutsideWhite { index: usize, value: u16 },
}

#[derive(Debug, Clone, PartialEq)]
pub enum RawLinearSource {
    XTrans {
        raw: RawMosaic,
        active_area: Option<Roi>,
        calibration: RawLinearCalibration,
        camera_identity: [u8; 32],
    },
    AlreadyLinearRgb {
        image: LinearRawRgbU16,
        calibration: Option<RawLinearCalibration>,
    },
}

impl RawLinearSource {
    #[must_use]
    pub const fn kind(&self) -> RawLinearSourceKind {
        match self {
            Self::XTrans { .. } => RawLinearSourceKind::XTrans,
            Self::AlreadyLinearRgb { .. } => RawLinearSourceKind::AlreadyLinearRgb,
        }
    }

    #[must_use]
    pub fn dimensions(&self) -> ImageDimensions {
        match self {
            Self::XTrans {
                raw, active_area, ..
            } => active_area
                .and_then(|roi| ImageDimensions::new(roi.width(), roi.height()).ok())
                .unwrap_or_else(|| raw.dimensions()),
            Self::AlreadyLinearRgb { image, .. } => image.dimensions(),
        }
    }

    pub fn source_identity(&self) -> [u8; 32] {
        match self {
            Self::XTrans {
                raw,
                active_area,
                calibration,
                camera_identity,
            } => {
                let mut hasher = Sha256::new();
                hasher.update(b"rusttable.raw-linear-xtrans-source-v1");
                hasher.update(source_identity(
                    RawLinearSourceKind::XTrans,
                    raw.dimensions(),
                    raw.samples(),
                    *camera_identity,
                    ColorEncoding::LinearRec2020D65,
                ));
                hasher.update(calibration.identity());
                if let Some(roi) = active_area {
                    write_roi(&mut hasher, *roi);
                }
                hasher.finalize().into()
            }
            Self::AlreadyLinearRgb { image, calibration } => {
                let mut hasher = Sha256::new();
                hasher.update(image.source_identity());
                if let Some(calibration) = calibration {
                    hasher.update(calibration.identity());
                }
                hasher.finalize().into()
            }
        }
    }

    #[must_use]
    pub fn calibration(&self) -> Option<&RawLinearCalibration> {
        match self {
            Self::XTrans { calibration, .. }
            | Self::AlreadyLinearRgb {
                calibration: Some(calibration),
                ..
            } => Some(calibration),
            Self::AlreadyLinearRgb {
                calibration: None, ..
            } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RawOperationKind {
    RawPrepare,
    Highlights,
    Demosaic,
    Exposure,
    ToneMapping,
    Sharpening,
    CreativeColor,
    OutputProfile,
    RawDenoise,
    AiRestoration,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawOperationSpec {
    pub id: u128,
    pub version: u16,
    pub kind: RawOperationKind,
    pub parameters: Vec<u8>,
    pub enabled: bool,
}

impl RawOperationSpec {
    pub fn new(
        id: u128,
        version: u16,
        kind: RawOperationKind,
        parameters: Vec<u8>,
        enabled: bool,
    ) -> Result<Self, RawOperationError> {
        if id == 0 || version == 0 || parameters.len() > 64 * 1024 {
            return Err(RawOperationError::Invalid);
        }
        Ok(Self {
            id,
            version,
            kind,
            parameters,
            enabled,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawOperationError {
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RawLinearEditSnapshot {
    revision: u64,
    operations: Vec<RawOperationSpec>,
}

impl RawLinearEditSnapshot {
    pub fn new(revision: u64, operations: Vec<RawOperationSpec>) -> Result<Self, RawEditError> {
        let mut ids = std::collections::BTreeSet::new();
        for operation in &operations {
            if !ids.insert(operation.id) {
                return Err(RawEditError::DuplicateOperation);
            }
        }
        Ok(Self {
            revision,
            operations,
        })
    }
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }
    #[must_use]
    pub fn operations(&self) -> &[RawOperationSpec] {
        &self.operations
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawEditError {
    DuplicateOperation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CollisionPolicy {
    UniqueSuffix,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawLinearOutputRequest {
    destination: PathBuf,
    add_to_catalog: bool,
    group_with_source: bool,
    collision: CollisionPolicy,
    quantization_white: u16,
    camera_identity: [u8; 32],
}

impl RawLinearOutputRequest {
    pub fn new(
        destination: PathBuf,
        camera_identity: [u8; 32],
    ) -> Result<Self, OutputRequestError> {
        if destination.file_name().is_none() || camera_identity == [0; 32] {
            return Err(OutputRequestError::Invalid);
        }
        Ok(Self {
            destination,
            add_to_catalog: true,
            group_with_source: true,
            collision: CollisionPolicy::UniqueSuffix,
            quantization_white: u16::MAX,
            camera_identity,
        })
    }
    #[must_use]
    pub fn destination(&self) -> &Path {
        &self.destination
    }
    #[must_use]
    pub const fn add_to_catalog(&self) -> bool {
        self.add_to_catalog
    }
    #[must_use]
    pub const fn group_with_source(&self) -> bool {
        self.group_with_source
    }
    #[must_use]
    pub const fn collision(&self) -> CollisionPolicy {
        self.collision
    }
    #[must_use]
    pub const fn quantization_white(&self) -> u16 {
        self.quantization_white
    }
    #[must_use]
    pub const fn camera_identity(&self) -> [u8; 32] {
        self.camera_identity
    }
    #[must_use]
    pub const fn with_catalog(mut self, add: bool, group: bool) -> Self {
        self.add_to_catalog = add;
        self.group_with_source = group;
        self
    }
    #[must_use]
    pub const fn with_collision(mut self, collision: CollisionPolicy) -> Self {
        self.collision = collision;
        self
    }
    #[must_use]
    pub const fn with_quantization_white(mut self, white: u16) -> Self {
        self.quantization_white = white;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputRequestError {
    Invalid,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawLinearDenoiseRequest {
    source: RawLinearSource,
    edit: RawLinearEditSnapshot,
    model_identity: [u8; 32],
    provider: crate::ProviderPolicy,
    strength: Strength,
    output: RawLinearOutputRequest,
    identity: [u8; 32],
}

impl RawLinearDenoiseRequest {
    pub fn new(
        source: RawLinearSource,
        edit: RawLinearEditSnapshot,
        model_identity: [u8; 32],
        provider: crate::ProviderPolicy,
        strength: Strength,
        output: RawLinearOutputRequest,
    ) -> Result<Self, RequestError> {
        if model_identity == [0; 32] {
            return Err(RequestError::InvalidModelIdentity);
        }
        let identity =
            request_identity(&source, &edit, model_identity, provider, strength, &output);
        Ok(Self {
            source,
            edit,
            model_identity,
            provider,
            strength,
            output,
            identity,
        })
    }
    #[must_use]
    pub const fn source(&self) -> &RawLinearSource {
        &self.source
    }
    #[must_use]
    pub const fn edit(&self) -> &RawLinearEditSnapshot {
        &self.edit
    }
    #[must_use]
    pub const fn model_identity(&self) -> [u8; 32] {
        self.model_identity
    }
    #[must_use]
    pub const fn provider(&self) -> crate::ProviderPolicy {
        self.provider
    }
    #[must_use]
    pub const fn strength(&self) -> Strength {
        self.strength
    }
    #[must_use]
    pub const fn output(&self) -> &RawLinearOutputRequest {
        &self.output
    }
    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestError {
    InvalidModelIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawLinearPlan {
    preparation_mode: RawLinearPreparationMode,
    source_identity: [u8; 32],
    edit_identity: [u8; 32],
    model_identity: [u8; 32],
    source_kind: RawLinearSourceKind,
    source_dimensions: ImageDimensions,
    included: Vec<RawOperationSpec>,
    excluded: Vec<RawOperationSpec>,
    color_plan: TransformPlan,
    tiles: Vec<RawLinearTile>,
    strength: Strength,
    identity: [u8; 32],
}

impl RawLinearPlan {
    pub(crate) fn new(
        source_identity: [u8; 32],
        edit_identity: [u8; 32],
        model_identity: [u8; 32],
        source_kind: RawLinearSourceKind,
        source_dimensions: ImageDimensions,
        included: Vec<RawOperationSpec>,
        excluded: Vec<RawOperationSpec>,
        color_plan: TransformPlan,
        tiles: Vec<RawLinearTile>,
        strength: Strength,
    ) -> Result<Self, PlanIdentityError> {
        let mut hasher = Sha256::new();
        hasher.update(b"rusttable.raw-linear-plan-v1");
        hasher.update(
            RawLinearPreparationMode::AiRawLinearPreparation
                .tag()
                .as_bytes(),
        );
        hasher.update(source_identity);
        hasher.update(edit_identity);
        hasher.update(model_identity);
        hasher.update([source_kind as u8]);
        hasher.update(source_dimensions.width().to_le_bytes());
        hasher.update(source_dimensions.height().to_le_bytes());
        hasher.update(
            color_plan
                .identity()
                .map_err(|_| PlanIdentityError::ColorPlan)?,
        );
        for operation in included.iter().chain(excluded.iter()) {
            hasher.update(operation.id.to_le_bytes());
            hasher.update(operation.version.to_le_bytes());
            hasher.update([operation.kind as u8, u8::from(operation.enabled)]);
            hasher.update(&operation.parameters);
        }
        for tile in &tiles {
            tile.hash_into(&mut hasher);
        }
        hasher.update(strength.0.to_le_bytes());
        Ok(Self {
            preparation_mode: RawLinearPreparationMode::AiRawLinearPreparation,
            source_identity,
            edit_identity,
            model_identity,
            source_kind,
            source_dimensions,
            included,
            excluded,
            color_plan,
            tiles,
            strength,
            identity: hasher.finalize().into(),
        })
    }
    #[must_use]
    pub const fn preparation_mode(&self) -> RawLinearPreparationMode {
        self.preparation_mode
    }
    #[must_use]
    pub const fn source_identity(&self) -> [u8; 32] {
        self.source_identity
    }
    #[must_use]
    pub const fn edit_identity(&self) -> [u8; 32] {
        self.edit_identity
    }
    #[must_use]
    pub const fn model_identity(&self) -> [u8; 32] {
        self.model_identity
    }
    #[must_use]
    pub const fn source_kind(&self) -> RawLinearSourceKind {
        self.source_kind
    }
    #[must_use]
    pub const fn source_dimensions(&self) -> ImageDimensions {
        self.source_dimensions
    }
    #[must_use]
    pub fn included_operations(&self) -> &[RawOperationSpec] {
        &self.included
    }
    #[must_use]
    pub fn excluded_operations(&self) -> &[RawOperationSpec] {
        &self.excluded
    }
    #[must_use]
    pub(crate) fn color_plan(&self) -> &TransformPlan {
        &self.color_plan
    }
    #[must_use]
    pub fn tiles(&self) -> &[RawLinearTile] {
        &self.tiles
    }
    #[must_use]
    pub const fn strength(&self) -> Strength {
        self.strength
    }
    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RawLinearTile {
    input_x: u32,
    input_y: u32,
    width: u32,
    height: u32,
    output_x: u32,
    output_y: u32,
    output_width: u32,
    output_height: u32,
    overlap: u32,
}

impl RawLinearTile {
    pub(crate) const fn new(
        input_x: u32,
        input_y: u32,
        width: u32,
        height: u32,
        output_x: u32,
        output_y: u32,
        output_width: u32,
        output_height: u32,
        overlap: u32,
    ) -> Self {
        Self {
            input_x,
            input_y,
            width,
            height,
            output_x,
            output_y,
            output_width,
            output_height,
            overlap,
        }
    }
    #[must_use]
    pub const fn input_origin(self) -> (u32, u32) {
        (self.input_x, self.input_y)
    }
    #[must_use]
    pub const fn input_dimensions(self) -> (u32, u32) {
        (self.width, self.height)
    }
    #[must_use]
    pub const fn output_origin(self) -> (u32, u32) {
        (self.output_x, self.output_y)
    }
    #[must_use]
    pub const fn output_dimensions(self) -> (u32, u32) {
        (self.output_width, self.output_height)
    }
    #[must_use]
    pub const fn overlap(self) -> u32 {
        self.overlap
    }
    fn hash_into(self, hasher: &mut Sha256) {
        for value in [
            self.input_x,
            self.input_y,
            self.width,
            self.height,
            self.output_x,
            self.output_y,
            self.output_width,
            self.output_height,
            self.overlap,
        ] {
            hasher.update(value.to_le_bytes());
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanIdentityError {
    ColorPlan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawLinearReceipt {
    pub workflow_version: u16,
    pub request_identity: [u8; 32],
    pub source_identity: [u8; 32],
    pub edit_identity: [u8; 32],
    pub plan_identity: [u8; 32],
    pub model_identity: [u8; 32],
    pub provider: crate::Provider,
    pub tile_count: u64,
    pub strength_millis: u16,
    pub output_identity: [u8; 32],
    pub imported: bool,
    pub grouped: bool,
}

pub(crate) fn write_roi(hasher: &mut Sha256, roi: Roi) {
    for value in [roi.x(), roi.y(), roi.width(), roi.height()] {
        hasher.update(value.to_le_bytes());
    }
}

fn source_identity(
    kind: RawLinearSourceKind,
    dimensions: ImageDimensions,
    samples: &[u16],
    camera_identity: [u8; 32],
    color: ColorEncoding,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.raw-linear-source-v1");
    hasher.update([kind as u8]);
    hasher.update(dimensions.width().to_le_bytes());
    hasher.update(dimensions.height().to_le_bytes());
    hasher.update(camera_identity);
    hasher.update(format!("{color:?}").as_bytes());
    for sample in samples {
        hasher.update(sample.to_le_bytes());
    }
    hasher.finalize().into()
}

pub(crate) fn edit_identity(edit: &RawLinearEditSnapshot) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.raw-linear-edit-v1");
    hasher.update(edit.revision.to_le_bytes());
    for operation in &edit.operations {
        hasher.update(operation.id.to_le_bytes());
        hasher.update(operation.version.to_le_bytes());
        hasher.update([operation.kind as u8, u8::from(operation.enabled)]);
        hasher.update(&operation.parameters);
    }
    hasher.finalize().into()
}

fn request_identity(
    source: &RawLinearSource,
    edit: &RawLinearEditSnapshot,
    model_identity: [u8; 32],
    provider: crate::ProviderPolicy,
    strength: Strength,
    output: &RawLinearOutputRequest,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.raw-linear-request-v1");
    hasher.update(source.source_identity());
    hasher.update(edit_identity(edit));
    hasher.update(model_identity);
    hasher.update([provider.selected() as u8]);
    hasher.update(strength.0.to_le_bytes());
    hasher.update([
        output.collision as u8,
        u8::from(output.add_to_catalog),
        u8::from(output.group_with_source),
    ]);
    hasher.update(output.quantization_white.to_le_bytes());
    hasher.update(output.camera_identity);
    hasher.update(output.destination.as_os_str().to_string_lossy().as_bytes());
    hasher.finalize().into()
}

impl fmt::Display for StrengthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("raw-linear strength must be finite and in 0..=1")
    }
}
impl std::error::Error for StrengthError {}

impl fmt::Display for CalibrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RAW calibration is invalid or singular")
    }
}
impl std::error::Error for CalibrationError {}
impl fmt::Display for LinearRawRgbU16Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid LinearRawRgbU16: {self:?}")
    }
}
impl std::error::Error for LinearRawRgbU16Error {}
impl fmt::Display for RawOperationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid RAW operation")
    }
}
impl std::error::Error for RawOperationError {}
impl fmt::Display for RawEditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RAW edit snapshot has duplicate operations")
    }
}
impl std::error::Error for RawEditError {}
impl fmt::Display for OutputRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid RAW output request")
    }
}
impl std::error::Error for OutputRequestError {}
impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid RAW-linear request")
    }
}
impl std::error::Error for RequestError {}
