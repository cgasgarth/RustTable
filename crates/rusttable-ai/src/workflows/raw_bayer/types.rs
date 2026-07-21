use std::fmt;
use std::path::{Path, PathBuf};

use rusttable_color::Matrix3;
use rusttable_image::{
    CfaColor, CfaPattern, CfaPhase, ImageDimensions, Orientation, RawMosaic, Roi,
};
use sha2::{Digest, Sha256};

pub const RAW_BAYER_WORKFLOW_VERSION: u16 = 1;
pub const RAW_BAYER_PLAN_VERSION: u16 = 1;

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

#[derive(Debug, Clone, PartialEq)]
pub struct BayerCalibration {
    identity: [u8; 32],
    black: [u16; 4],
    white: [u16; 4],
    white_balance: [f32; 4],
    camera_to_xyz: Matrix3,
}

impl BayerCalibration {
    pub fn new(
        identity: [u8; 32],
        black: [u16; 4],
        white: [u16; 4],
        white_balance: [f32; 4],
        camera_to_xyz: Matrix3,
    ) -> Result<Self, CalibrationError> {
        if identity == [0; 32]
            || black
                .iter()
                .zip(white)
                .any(|(black, white)| white <= *black)
            || white_balance
                .iter()
                .any(|gain| !gain.is_finite() || *gain <= 0.0)
            || camera_to_xyz.inverse().is_err()
        {
            return Err(CalibrationError::Invalid);
        }
        let green = f32::midpoint(white_balance[1], white_balance[2]);
        if !green.is_finite() || green <= 0.0 {
            return Err(CalibrationError::Invalid);
        }
        Ok(Self {
            identity,
            black,
            white,
            white_balance,
            camera_to_xyz,
        })
    }

    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }
    #[must_use]
    pub const fn black(&self) -> [u16; 4] {
        self.black
    }
    #[must_use]
    pub const fn white(&self) -> [u16; 4] {
        self.white
    }
    #[must_use]
    pub const fn white_balance(&self) -> [f32; 4] {
        self.white_balance
    }
    #[must_use]
    pub const fn camera_to_xyz(&self) -> Matrix3 {
        self.camera_to_xyz
    }

    #[must_use]
    pub fn normalized_gains(&self) -> [f32; 4] {
        let green = f32::midpoint(self.white_balance[1], self.white_balance[2]);
        self.white_balance.map(|gain| gain / green)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalibrationError {
    Invalid,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawFrame {
    raw: RawMosaic,
    active_area: Option<Roi>,
    default_crop: Option<Roi>,
    masked_areas: Vec<Roi>,
    calibration: BayerCalibration,
    identity: [u8; 32],
}

impl RawFrame {
    pub fn new(
        raw: RawMosaic,
        active_area: Option<Roi>,
        default_crop: Option<Roi>,
        masked_areas: Vec<Roi>,
        calibration: BayerCalibration,
    ) -> Result<Self, RawFrameError> {
        validate_bayer(raw.pattern())?;
        let dimensions = raw.dimensions();
        if let Some(roi) = active_area {
            validate_roi(roi, dimensions)?;
        }
        if let Some(roi) = default_crop {
            validate_roi(roi, dimensions)?;
            if let Some(active) = active_area
                && (roi.x() < active.x()
                    || roi.y() < active.y()
                    || roi.bottom() > active.bottom()
                    || roi.right() > active.right())
            {
                return Err(RawFrameError::DefaultCropOutsideActiveArea);
            }
        }
        for roi in &masked_areas {
            validate_roi(*roi, dimensions)?;
        }
        for y in 0..dimensions.height() {
            for x in 0..dimensions.width() {
                let plane = plane_for(raw.pattern(), raw.phase(), x, y);
                let index = usize::try_from(y)
                    .ok()
                    .and_then(|row| row.checked_mul(raw.row_stride_samples()))
                    .and_then(|index| index.checked_add(usize::try_from(x).ok()?))
                    .ok_or(RawFrameError::ArithmeticOverflow)?;
                if raw.samples()[index] > calibration.white()[plane] {
                    return Err(RawFrameError::SampleOutsideWhite { index });
                }
            }
        }
        let identity = frame_identity(&raw, active_area, default_crop, &masked_areas, &calibration);
        Ok(Self {
            raw,
            active_area,
            default_crop,
            masked_areas,
            calibration,
            identity,
        })
    }

    #[must_use]
    pub const fn raw(&self) -> &RawMosaic {
        &self.raw
    }
    #[must_use]
    pub const fn active_area(&self) -> Option<Roi> {
        self.active_area
    }
    #[must_use]
    pub const fn default_crop(&self) -> Option<Roi> {
        self.default_crop
    }
    #[must_use]
    pub fn masked_areas(&self) -> &[Roi] {
        &self.masked_areas
    }
    #[must_use]
    pub const fn calibration(&self) -> &BayerCalibration {
        &self.calibration
    }
    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }

    #[must_use]
    pub fn processing_area(&self) -> Roi {
        self.active_area
            .unwrap_or_else(|| Roi::full(self.raw.dimensions()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawFrameError {
    UnsupportedCfa,
    InvalidRoi,
    DefaultCropOutsideActiveArea,
    ArithmeticOverflow,
    SampleOutsideWhite { index: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RawBayerEditSnapshot {
    revision: u64,
    operations: Vec<RawBayerOperation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawBayerOperation {
    pub id: u128,
    pub version: u16,
    pub parameters: Vec<u8>,
    pub enabled: bool,
}

impl RawBayerEditSnapshot {
    pub fn new(revision: u64, operations: Vec<RawBayerOperation>) -> Result<Self, EditError> {
        let mut ids = std::collections::BTreeSet::new();
        if operations
            .iter()
            .any(|op| op.id == 0 || op.version == 0 || op.parameters.len() > 64 * 1024)
        {
            return Err(EditError::InvalidOperation);
        }
        if operations.iter().any(|op| !ids.insert(op.id)) {
            return Err(EditError::DuplicateOperation);
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
    pub fn operations(&self) -> &[RawBayerOperation] {
        &self.operations
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditError {
    DuplicateOperation,
    InvalidOperation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CollisionPolicy {
    UniqueSuffix,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawBayerOutputRequest {
    destination: PathBuf,
    camera_identity: [u8; 32],
    add_to_catalog: bool,
    group_with_source: bool,
    collision: CollisionPolicy,
}

impl RawBayerOutputRequest {
    pub fn new(
        destination: PathBuf,
        camera_identity: [u8; 32],
    ) -> Result<Self, OutputRequestError> {
        if destination.file_name().is_none() || camera_identity == [0; 32] {
            return Err(OutputRequestError::Invalid);
        }
        Ok(Self {
            destination,
            camera_identity,
            add_to_catalog: true,
            group_with_source: true,
            collision: CollisionPolicy::UniqueSuffix,
        })
    }
    #[must_use]
    pub fn destination(&self) -> &Path {
        &self.destination
    }
    #[must_use]
    pub const fn camera_identity(&self) -> [u8; 32] {
        self.camera_identity
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputRequestError {
    Invalid,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawBayerDenoiseRequest {
    source: RawFrame,
    edit: RawBayerEditSnapshot,
    model_identity: [u8; 32],
    provider: crate::ProviderPolicy,
    strength: Strength,
    output: RawBayerOutputRequest,
    identity: [u8; 32],
}

impl RawBayerDenoiseRequest {
    pub fn new(
        source: RawFrame,
        edit: RawBayerEditSnapshot,
        model_identity: [u8; 32],
        provider: crate::ProviderPolicy,
        strength: Strength,
        output: RawBayerOutputRequest,
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
    pub const fn source(&self) -> &RawFrame {
        &self.source
    }
    #[must_use]
    pub const fn edit(&self) -> &RawBayerEditSnapshot {
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
    pub const fn output(&self) -> &RawBayerOutputRequest {
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
pub struct RawBayerTile {
    input_x: u32,
    input_y: u32,
    width: u32,
    height: u32,
    output_x: u32,
    output_y: u32,
    output_width: u32,
    output_height: u32,
}

impl RawBayerTile {
    pub(crate) const fn new(
        input_x: u32,
        input_y: u32,
        width: u32,
        height: u32,
        output_x: u32,
        output_y: u32,
        output_width: u32,
        output_height: u32,
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
        }
    }
    #[must_use]
    pub const fn input_origin(&self) -> (u32, u32) {
        (self.input_x, self.input_y)
    }
    #[must_use]
    pub const fn input_dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    #[must_use]
    pub const fn output_origin(&self) -> (u32, u32) {
        (self.output_x, self.output_y)
    }
    #[must_use]
    pub const fn output_dimensions(&self) -> (u32, u32) {
        (self.output_width, self.output_height)
    }
    fn hash_into(&self, hasher: &mut Sha256) {
        for value in [
            self.input_x,
            self.input_y,
            self.width,
            self.height,
            self.output_x,
            self.output_y,
            self.output_width,
            self.output_height,
        ] {
            hasher.update(value.to_le_bytes());
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawBayerPlan {
    source_identity: [u8; 32],
    edit_identity: [u8; 32],
    model_identity: [u8; 32],
    processing_area: Roi,
    packed_dimensions: ImageDimensions,
    tiles: Vec<RawBayerTile>,
    identity: [u8; 32],
}

impl RawBayerPlan {
    pub(crate) fn new(
        request: &RawBayerDenoiseRequest,
        packed_dimensions: ImageDimensions,
        tiles: Vec<RawBayerTile>,
    ) -> Self {
        let source_identity = request.source.identity();
        let edit_identity = edit_identity(request.edit());
        let mut hasher = Sha256::new();
        hasher.update(b"rusttable.raw-bayer-plan-v1");
        hasher.update(source_identity);
        hasher.update(edit_identity);
        hasher.update(request.model_identity());
        hasher.update(request.source.processing_area().x().to_le_bytes());
        hasher.update(request.source.processing_area().y().to_le_bytes());
        hasher.update(request.source.processing_area().width().to_le_bytes());
        hasher.update(request.source.processing_area().height().to_le_bytes());
        hasher.update(packed_dimensions.width().to_le_bytes());
        hasher.update(packed_dimensions.height().to_le_bytes());
        for tile in &tiles {
            tile.hash_into(&mut hasher);
        }
        hasher.update(request.strength().0.to_le_bytes());
        Self {
            source_identity,
            edit_identity,
            model_identity: request.model_identity(),
            processing_area: request.source.processing_area(),
            packed_dimensions,
            tiles,
            identity: hasher.finalize().into(),
        }
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
    pub const fn processing_area(&self) -> Roi {
        self.processing_area
    }
    #[must_use]
    pub const fn packed_dimensions(&self) -> ImageDimensions {
        self.packed_dimensions
    }
    #[must_use]
    pub fn tiles(&self) -> &[RawBayerTile] {
        &self.tiles
    }
    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CfaBayerU16 {
    dimensions: ImageDimensions,
    row_stride_samples: usize,
    samples: Vec<u16>,
    pattern: CfaPattern,
    phase: CfaPhase,
    orientation: Orientation,
    active_area: Option<Roi>,
    default_crop: Option<Roi>,
    masked_areas: Vec<Roi>,
    calibration: BayerCalibration,
    source_identity: [u8; 32],
    output_identity: [u8; 32],
}

impl CfaBayerU16 {
    pub fn new(frame: &RawFrame, samples: Vec<u16>) -> Result<Self, CfaBayerU16Error> {
        let raw = frame.raw();
        if samples.len() != raw.samples().len() {
            return Err(CfaBayerU16Error::SampleLength);
        }
        for y in 0..raw.dimensions().height() {
            for x in 0..raw.dimensions().width() {
                let plane = plane_for(raw.pattern(), raw.phase(), x, y);
                let index = usize::try_from(y)
                    .ok()
                    .and_then(|row| row.checked_mul(raw.row_stride_samples()))
                    .and_then(|i| i.checked_add(usize::try_from(x).ok()?))
                    .ok_or(CfaBayerU16Error::ArithmeticOverflow)?;
                if samples[index] < frame.calibration().black()[plane]
                    || samples[index] > frame.calibration().white()[plane]
                {
                    return Err(CfaBayerU16Error::SampleOutsideRange { index });
                }
            }
        }
        let mut hasher = Sha256::new();
        hasher.update(b"rusttable.cfa-bayer-u16-v1");
        hasher.update(frame.identity());
        for sample in &samples {
            hasher.update(sample.to_le_bytes());
        }
        Ok(Self {
            dimensions: raw.dimensions(),
            row_stride_samples: raw.row_stride_samples(),
            samples,
            pattern: raw.pattern(),
            phase: raw.phase(),
            orientation: raw.orientation(),
            active_area: frame.active_area(),
            default_crop: frame.default_crop(),
            masked_areas: frame.masked_areas().to_vec(),
            calibration: frame.calibration().clone(),
            source_identity: frame.identity(),
            output_identity: hasher.finalize().into(),
        })
    }
    #[must_use]
    pub const fn dimensions(&self) -> ImageDimensions {
        self.dimensions
    }
    #[must_use]
    pub const fn row_stride_samples(&self) -> usize {
        self.row_stride_samples
    }
    #[must_use]
    pub fn samples(&self) -> &[u16] {
        &self.samples
    }
    #[must_use]
    pub const fn pattern(&self) -> CfaPattern {
        self.pattern
    }
    #[must_use]
    pub const fn phase(&self) -> CfaPhase {
        self.phase
    }
    #[must_use]
    pub const fn orientation(&self) -> Orientation {
        self.orientation
    }
    #[must_use]
    pub const fn active_area(&self) -> Option<Roi> {
        self.active_area
    }
    #[must_use]
    pub const fn default_crop(&self) -> Option<Roi> {
        self.default_crop
    }
    #[must_use]
    pub fn masked_areas(&self) -> &[Roi] {
        &self.masked_areas
    }
    #[must_use]
    pub const fn calibration(&self) -> &BayerCalibration {
        &self.calibration
    }
    #[must_use]
    pub const fn source_identity(&self) -> [u8; 32] {
        self.source_identity
    }
    #[must_use]
    pub const fn output_identity(&self) -> [u8; 32] {
        self.output_identity
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CfaBayerU16Error {
    SampleLength,
    ArithmeticOverflow,
    SampleOutsideRange { index: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawBayerReceipt {
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

pub(crate) fn validate_bayer(pattern: CfaPattern) -> Result<(), RawFrameError> {
    let CfaPattern::Bayer(pattern) = pattern else {
        return Err(RawFrameError::UnsupportedCfa);
    };
    let mut red = 0;
    let mut blue = 0;
    let mut green = 0;
    for row in pattern {
        for color in row {
            match color {
                CfaColor::Red => red += 1,
                CfaColor::Green => green += 1,
                CfaColor::Blue => blue += 1,
                CfaColor::Clear => {}
            }
        }
    }
    if red != 1 || blue != 1 || green != 2 {
        return Err(RawFrameError::UnsupportedCfa);
    }
    Ok(())
}

pub(crate) fn plane_offsets(pattern: CfaPattern, phase: CfaPhase) -> [(u32, u32); 4] {
    let mut offsets = [(0, 0); 4];
    let mut green = 1;
    for y in 0..2 {
        for x in 0..2 {
            let plane = match pattern.color_at(x, y, phase) {
                CfaColor::Red | CfaColor::Clear => 0,
                CfaColor::Blue => 3,
                CfaColor::Green => {
                    let value = green;
                    green += 1;
                    value
                }
            };
            offsets[plane] = (x, y);
        }
    }
    offsets
}

pub(crate) fn plane_for(pattern: CfaPattern, phase: CfaPhase, x: u32, y: u32) -> usize {
    let offsets = plane_offsets(pattern, phase);
    let dx = x % 2;
    let dy = y % 2;
    offsets
        .iter()
        .position(|(x, y)| *x == dx && *y == dy)
        .unwrap_or(0)
}

fn validate_roi(roi: Roi, dimensions: ImageDimensions) -> Result<(), RawFrameError> {
    if roi.is_empty() {
        return Err(RawFrameError::InvalidRoi);
    }
    roi.within(dimensions)
        .map(|_| ())
        .map_err(|_| RawFrameError::InvalidRoi)
}

fn frame_identity(
    raw: &RawMosaic,
    active: Option<Roi>,
    crop: Option<Roi>,
    masked: &[Roi],
    calibration: &BayerCalibration,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.raw-bayer-source-v1");
    hasher.update(raw.dimensions().width().to_le_bytes());
    hasher.update(raw.dimensions().height().to_le_bytes());
    hasher.update(raw.row_stride_samples().to_le_bytes());
    hasher.update([
        raw.orientation() as u8,
        raw.phase().x() as u8,
        raw.phase().y() as u8,
    ]);
    for sample in raw.samples() {
        hasher.update(sample.to_le_bytes());
    }
    hasher.update(calibration.identity());
    for roi in active.into_iter().chain(crop).chain(masked.iter().copied()) {
        for value in [roi.x(), roi.y(), roi.width(), roi.height()] {
            hasher.update(value.to_le_bytes());
        }
    }
    hasher.finalize().into()
}

fn edit_identity(edit: &RawBayerEditSnapshot) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.raw-bayer-edit-v1");
    hasher.update(edit.revision.to_le_bytes());
    for op in &edit.operations {
        hasher.update(op.id.to_le_bytes());
        hasher.update(op.version.to_le_bytes());
        hasher.update([u8::from(op.enabled)]);
        hasher.update(&op.parameters);
    }
    hasher.finalize().into()
}

fn request_identity(
    source: &RawFrame,
    edit: &RawBayerEditSnapshot,
    model: [u8; 32],
    provider: crate::ProviderPolicy,
    strength: Strength,
    output: &RawBayerOutputRequest,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.raw-bayer-request-v1");
    hasher.update(source.identity());
    hasher.update(edit_identity(edit));
    hasher.update(model);
    hasher.update([
        provider.selected() as u8,
        output.collision as u8,
        u8::from(output.add_to_catalog),
        u8::from(output.group_with_source),
    ]);
    hasher.update(strength.0.to_le_bytes());
    hasher.update(output.camera_identity);
    hasher.update(output.destination.as_os_str().to_string_lossy().as_bytes());
    hasher.finalize().into()
}

impl fmt::Display for StrengthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Bayer denoise strength must be finite and in 0..=1")
    }
}
impl std::error::Error for StrengthError {}
impl fmt::Display for CalibrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Bayer calibration is incomplete or invalid")
    }
}
impl std::error::Error for CalibrationError {}
impl fmt::Display for RawFrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid Bayer RAW frame: {self:?}")
    }
}
impl std::error::Error for RawFrameError {}
impl fmt::Display for CfaBayerU16Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid CfaBayerU16 output: {self:?}")
    }
}
impl std::error::Error for CfaBayerU16Error {}
impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid Bayer edit snapshot: {self:?}")
    }
}
impl std::error::Error for EditError {}
impl fmt::Display for OutputRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid Bayer output request")
    }
}
impl std::error::Error for OutputRequestError {}
impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid Bayer workflow request")
    }
}
impl std::error::Error for RequestError {}
