#![expect(
    clippy::missing_errors_doc,
    clippy::struct_excessive_bools,
    reason = "the workflow contract documents external ICC/model terms and explicit state flags"
)]

use std::{fmt, path::PathBuf};

use rusttable_pixelpipe::{RgbaF32Image, SourceRasterIdentity};
use sha2::{Digest, Sha256};

pub const WORKFLOW_VERSION: u32 = 1;
pub const DETAIL_BAND_MULTIPLIERS: [f32; 5] = [0.25, 0.15, 0.05, 0.02, 0.01];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matrix3([f32; 9]);

impl Matrix3 {
    #[must_use]
    pub const fn identity() -> Self {
        Self([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0])
    }

    #[must_use]
    pub const fn from_rows(rows: [[f32; 3]; 3]) -> Self {
        Self([
            rows[0][0], rows[0][1], rows[0][2], rows[1][0], rows[1][1], rows[1][2], rows[2][0],
            rows[2][1], rows[2][2],
        ])
    }

    #[must_use]
    pub const fn as_array(self) -> [f32; 9] {
        self.0
    }

    #[must_use]
    pub fn apply(self, red: f32, green: f32, blue: f32) -> [f32; 3] {
        [
            self.0[0] * red + self.0[1] * green + self.0[2] * blue,
            self.0[3] * red + self.0[4] * green + self.0[5] * blue,
            self.0[6] * red + self.0[7] * green + self.0[8] * blue,
        ]
    }

    fn is_finite(self) -> bool {
        self.0.iter().all(|value| value.is_finite())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RgbProfile {
    identity: String,
    working_to_model: Matrix3,
    model_to_working: Matrix3,
    working_to_output: Matrix3,
    icc_profile: Vec<u8>,
}

impl RgbProfile {
    /// Creates a profile with explicit transforms and an ICC payload.
    ///
    /// The payload is retained verbatim for the TIFF ICC tag. `RustTable` does
    /// not substitute an sRGB payload when it is absent.
    pub fn new(
        identity: impl Into<String>,
        working_to_model: Matrix3,
        model_to_working: Matrix3,
        working_to_output: Matrix3,
        icc_profile: Vec<u8>,
    ) -> Result<Self, ProfileError> {
        let identity = identity.into();
        if identity.is_empty() {
            return Err(ProfileError::EmptyIdentity);
        }
        if identity.len() > 128 {
            return Err(ProfileError::IdentityTooLong);
        }
        if !working_to_model.is_finite()
            || !model_to_working.is_finite()
            || !working_to_output.is_finite()
        {
            return Err(ProfileError::NonFiniteTransform);
        }
        if icc_profile.is_empty() {
            return Err(ProfileError::MissingIccPayload);
        }
        if icc_profile.len() > 16 * 1024 * 1024 {
            return Err(ProfileError::IccPayloadTooLarge);
        }
        Ok(Self {
            identity,
            working_to_model,
            model_to_working,
            working_to_output,
            icc_profile,
        })
    }

    #[must_use]
    pub fn identity(&self) -> &str {
        &self.identity
    }

    #[must_use]
    pub const fn working_to_model(&self) -> Matrix3 {
        self.working_to_model
    }

    #[must_use]
    pub const fn model_to_working(&self) -> Matrix3 {
        self.model_to_working
    }

    #[must_use]
    pub const fn working_to_output(&self) -> Matrix3 {
        self.working_to_output
    }

    #[must_use]
    pub fn icc_profile(&self) -> &[u8] {
        &self.icc_profile
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileError {
    EmptyIdentity,
    IdentityTooLong,
    NonFiniteTransform,
    MissingIccPayload,
    IccPayloadTooLarge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Strength(u8);

impl Strength {
    /// Creates the Darktable-compatible inclusive `0..=100` strength.
    pub const fn new(value: u8) -> Result<Self, StrengthError> {
        if value <= 100 {
            Ok(Self(value))
        } else {
            Err(StrengthError::OutOfRange { value })
        }
    }

    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }

    #[must_use]
    pub const fn detail_recovery_strength(self) -> u8 {
        100 - self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrengthError {
    OutOfRange { value: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelTask {
    RgbDenoise,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelDescriptor {
    model_id: String,
    task: ModelTask,
    scale: u8,
    tile_size: u32,
    overlap: u32,
    qualified: bool,
    shadow_boost: bool,
}

impl ModelDescriptor {
    /// Describes one fixed-shape, qualified scale-1 model.
    pub fn new(
        model_id: impl Into<String>,
        task: ModelTask,
        scale: u8,
        tile_size: u32,
        overlap: u32,
        qualified: bool,
        shadow_boost: bool,
    ) -> Result<Self, ModelError> {
        let model_id = model_id.into();
        if model_id.is_empty() {
            return Err(ModelError::EmptyId);
        }
        if tile_size == 0 {
            return Err(ModelError::ZeroTileSize);
        }
        if overlap.saturating_mul(2) >= tile_size {
            return Err(ModelError::InvalidOverlap);
        }
        Ok(Self {
            model_id,
            task,
            scale,
            tile_size,
            overlap,
            qualified,
            shadow_boost,
        })
    }

    #[must_use]
    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    #[must_use]
    pub const fn task(&self) -> ModelTask {
        self.task
    }

    #[must_use]
    pub const fn scale(&self) -> u8 {
        self.scale
    }

    #[must_use]
    pub const fn tile_size(&self) -> u32 {
        self.tile_size
    }

    #[must_use]
    pub const fn overlap(&self) -> u32 {
        self.overlap
    }

    #[must_use]
    pub const fn qualified(&self) -> bool {
        self.qualified
    }

    #[must_use]
    pub const fn shadow_boost(&self) -> bool {
        self.shadow_boost
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelError {
    EmptyId,
    WrongTask,
    UnsupportedScale { scale: u8 },
    ZeroTileSize,
    InvalidOverlap,
    Unqualified,
    InvalidTileOutput,
    ProviderFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderSelection {
    Auto,
    Cpu,
    Gpu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderUsed {
    Cpu,
    Gpu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GamutPolicy {
    ConvertToWorking,
    PreserveWideGamut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShadowPolicy {
    Disabled,
    ProtectDeepShadows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DetailRecoveryPolicy {
    Disabled,
    Recover { strength: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TilePolicy {
    pub width: u32,
    pub height: u32,
    pub overlap: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MemoryEstimate {
    pub bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutputBitDepth {
    Eight,
    Sixteen,
    ThirtyTwoFloat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlphaOutput {
    Opaque,
    PreserveStraight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TiffCompression {
    Uncompressed,
    DeflateBalanced,
    PackBits,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TiffRecipe {
    bit_depth: OutputBitDepth,
    alpha: AlphaOutput,
    compression: TiffCompression,
    max_encoded_bytes: u64,
}

impl TiffRecipe {
    #[must_use]
    pub const fn default_color_managed() -> Self {
        Self {
            bit_depth: OutputBitDepth::Sixteen,
            alpha: AlphaOutput::PreserveStraight,
            compression: TiffCompression::DeflateBalanced,
            max_encoded_bytes: 512 * 1024 * 1024,
        }
    }

    /// Creates a checked TIFF encoding recipe.
    pub const fn new(
        bit_depth: OutputBitDepth,
        alpha: AlphaOutput,
        compression: TiffCompression,
        max_encoded_bytes: u64,
    ) -> Result<Self, TiffRecipeError> {
        if max_encoded_bytes == 0 {
            return Err(TiffRecipeError::ZeroByteLimit);
        }
        Ok(Self {
            bit_depth,
            alpha,
            compression,
            max_encoded_bytes,
        })
    }

    #[must_use]
    pub const fn bit_depth(&self) -> OutputBitDepth {
        self.bit_depth
    }

    #[must_use]
    pub const fn alpha(&self) -> AlphaOutput {
        self.alpha
    }

    #[must_use]
    pub const fn compression(&self) -> TiffCompression {
        self.compression
    }

    #[must_use]
    pub const fn max_encoded_bytes(&self) -> u64 {
        self.max_encoded_bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TiffRecipeError {
    ZeroByteLimit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CollisionPolicy {
    Fail,
    UniqueSuffix,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RgbDenoiseRequest {
    input: RgbaF32Image,
    render_identity: [u8; 32],
    working_profile: RgbProfile,
    output_profile: RgbProfile,
    strength: Strength,
    preserve_wide_gamut: bool,
    provider: ProviderSelection,
    tiff: TiffRecipe,
    destination: PathBuf,
    collision: CollisionPolicy,
    add_to_catalog: bool,
    group_with_source: bool,
}

impl RgbDenoiseRequest {
    /// Creates an immutable request. The caller must supply both color profiles.
    pub fn new(
        input: RgbaF32Image,
        render_identity: [u8; 32],
        working_profile: RgbProfile,
        output_profile: RgbProfile,
        destination: PathBuf,
    ) -> Result<Self, RequestError> {
        if destination.file_name().is_none() {
            return Err(RequestError::InvalidDestination);
        }
        Ok(Self {
            input,
            render_identity,
            working_profile,
            output_profile,
            strength: Strength(100),
            preserve_wide_gamut: true,
            provider: ProviderSelection::Auto,
            tiff: TiffRecipe::default_color_managed(),
            destination,
            collision: CollisionPolicy::UniqueSuffix,
            add_to_catalog: true,
            group_with_source: true,
        })
    }

    #[must_use]
    pub const fn input(&self) -> &RgbaF32Image {
        &self.input
    }

    #[must_use]
    pub const fn render_identity(&self) -> [u8; 32] {
        self.render_identity
    }

    #[must_use]
    pub fn working_profile(&self) -> &RgbProfile {
        &self.working_profile
    }

    #[must_use]
    pub fn output_profile(&self) -> &RgbProfile {
        &self.output_profile
    }

    #[must_use]
    pub const fn strength(&self) -> Strength {
        self.strength
    }

    #[must_use]
    pub const fn preserve_wide_gamut(&self) -> bool {
        self.preserve_wide_gamut
    }

    #[must_use]
    pub const fn provider(&self) -> ProviderSelection {
        self.provider
    }

    #[must_use]
    pub const fn tiff(&self) -> &TiffRecipe {
        &self.tiff
    }

    #[must_use]
    pub fn destination(&self) -> &std::path::Path {
        &self.destination
    }

    #[must_use]
    pub const fn collision(&self) -> CollisionPolicy {
        self.collision
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
    pub fn with_strength(mut self, strength: Strength) -> Self {
        self.strength = strength;
        self
    }

    #[must_use]
    pub const fn with_preserve_wide_gamut(mut self, value: bool) -> Self {
        self.preserve_wide_gamut = value;
        self
    }

    #[must_use]
    pub const fn with_provider(mut self, provider: ProviderSelection) -> Self {
        self.provider = provider;
        self
    }

    #[must_use]
    pub fn with_tiff(mut self, tiff: TiffRecipe) -> Self {
        self.tiff = tiff;
        self
    }

    #[must_use]
    pub const fn with_collision(mut self, collision: CollisionPolicy) -> Self {
        self.collision = collision;
        self
    }

    #[must_use]
    pub const fn with_catalog_import(
        mut self,
        add_to_catalog: bool,
        group_with_source: bool,
    ) -> Self {
        self.add_to_catalog = add_to_catalog;
        self.group_with_source = group_with_source;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestError {
    InvalidDestination,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RgbDenoiseStage {
    Validate,
    RenderSnapshot,
    Inference,
    DetailRecovery,
    ColorTransform,
    Encode,
    Publication,
    Probe,
    Import,
    Group,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RgbDenoiseProgress {
    pub stage: RgbDenoiseStage,
    pub completed: u64,
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbDenoisePlan {
    pub dimensions: rusttable_image::ImageDimensions,
    pub model_id: String,
    pub working_profile: String,
    pub output_profile: String,
    pub scale: u8,
    pub tile: TilePolicy,
    pub tile_count: u64,
    pub gamut_policy: GamutPolicy,
    pub shadow_policy: ShadowPolicy,
    pub detail_policy: DetailRecoveryPolicy,
    pub detail_recovery_strength: u8,
    pub preserve_wide_gamut: bool,
    pub memory: MemoryEstimate,
    pub identity: [u8; 32],
}

impl RgbDenoisePlan {
    /// Builds the immutable plan consumed by RGB denoise processing and UI/app
    /// dispatch. All values that affect processing are hashed into `identity`.
    pub fn build(
        width: u32,
        height: u32,
        model: &ModelDescriptor,
        working_profile: &RgbProfile,
        output_profile: &RgbProfile,
        strength: Strength,
        gamut_policy: GamutPolicy,
        shadow_policy: ShadowPolicy,
        detail_policy: DetailRecoveryPolicy,
    ) -> Result<Self, PlanError> {
        if width == 0 || height == 0 {
            return Err(PlanError::EmptyImage);
        }
        if model.task() != ModelTask::RgbDenoise {
            return Err(PlanError::WrongTask);
        }
        if model.scale() != 1 {
            return Err(PlanError::UnsupportedScale {
                scale: model.scale(),
            });
        }
        if !model.qualified() {
            return Err(PlanError::UnqualifiedModel);
        }
        if matches!(gamut_policy, GamutPolicy::PreserveWideGamut) && model.scale() != 1 {
            return Err(PlanError::WideGamutScale);
        }
        if matches!(shadow_policy, ShadowPolicy::ProtectDeepShadows) && !model.shadow_boost() {
            return Err(PlanError::ShadowUnsupported);
        }
        if let DetailRecoveryPolicy::Recover { strength } = detail_policy
            && strength > 100
        {
            return Err(PlanError::DetailStrength { strength });
        }
        let step = model
            .tile_size()
            .checked_sub(model.overlap().saturating_mul(2))
            .filter(|step| *step > 0)
            .ok_or(PlanError::InvalidTile)?;
        let tile_count = u64::from(width.div_ceil(step))
            .checked_mul(u64::from(height.div_ceil(step)))
            .ok_or(PlanError::Overflow)?;
        let pixels = u64::from(width)
            .checked_mul(u64::from(height))
            .ok_or(PlanError::Overflow)?;
        let source = pixels.checked_mul(16).ok_or(PlanError::Overflow)?;
        let output = source;
        let gamut = if matches!(gamut_policy, GamutPolicy::PreserveWideGamut) {
            pixels
        } else {
            0
        };
        let detail = if matches!(detail_policy, DetailRecoveryPolicy::Recover { .. }) {
            pixels.checked_mul(4).ok_or(PlanError::Overflow)?
        } else {
            0
        };
        let tile = u64::from(model.tile_size())
            .checked_mul(u64::from(model.tile_size()))
            .and_then(|pixels| pixels.checked_mul(3 * 4))
            .ok_or(PlanError::Overflow)?;
        let memory_bytes = source
            .checked_add(output)
            .and_then(|value| value.checked_add(gamut))
            .and_then(|value| value.checked_add(detail))
            .and_then(|value| value.checked_add(tile))
            .ok_or(PlanError::Overflow)?;
        if memory_bytes > 2 * 1024 * 1024 * 1024 {
            return Err(PlanError::MemoryLimit {
                bytes: memory_bytes,
            });
        }
        let dimensions = rusttable_image::ImageDimensions::new(width, height)
            .map_err(|_| PlanError::Overflow)?;
        let preserve_wide_gamut = matches!(gamut_policy, GamutPolicy::PreserveWideGamut);
        let detail_recovery_strength = match detail_policy {
            DetailRecoveryPolicy::Disabled => strength.detail_recovery_strength(),
            DetailRecoveryPolicy::Recover { strength } => strength,
        };
        let tile = TilePolicy {
            width: model.tile_size(),
            height: model.tile_size(),
            overlap: model.overlap(),
        };
        let mut hasher = Sha256::new();
        hasher.update(b"rusttable.ai.rgb-denoise.plan.v1");
        hasher.update(model.model_id().as_bytes());
        hasher.update(working_profile.identity().as_bytes());
        hasher.update(output_profile.identity().as_bytes());
        hasher.update([model.scale(), strength.get(), detail_recovery_strength]);
        hasher.update(model.tile_size().to_le_bytes());
        hasher.update(model.overlap().to_le_bytes());
        hasher.update([u8::from(preserve_wide_gamut)]);
        hasher.update([match shadow_policy {
            ShadowPolicy::Disabled => 0,
            ShadowPolicy::ProtectDeepShadows => 1,
        }]);
        let identity = hasher.finalize().into();
        Ok(Self {
            dimensions,
            model_id: model.model_id().to_owned(),
            working_profile: working_profile.identity().to_owned(),
            output_profile: output_profile.identity().to_owned(),
            scale: model.scale(),
            tile,
            tile_count,
            gamut_policy,
            shadow_policy,
            detail_policy,
            detail_recovery_strength,
            preserve_wide_gamut,
            memory: MemoryEstimate {
                bytes: memory_bytes,
            },
            identity,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanError {
    EmptyImage,
    WrongTask,
    UnsupportedScale { scale: u8 },
    UnqualifiedModel,
    WideGamutScale,
    ShadowUnsupported,
    DetailStrength { strength: u8 },
    InvalidTile,
    Overflow,
    MemoryLimit { bytes: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbDenoiseReceipt {
    pub workflow_version: u32,
    pub artifact_key: [u8; 32],
    pub destination: PathBuf,
    pub dimensions: rusttable_image::ImageDimensions,
    pub provider: ProviderUsed,
    pub detail_recovery_strength: u8,
    pub preserve_wide_gamut: bool,
    pub shadow_boost: bool,
    pub tile_count: u64,
    pub imported: bool,
    pub grouped: bool,
    pub source_identity: SourceRasterIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbDenoiseBatchReceipt {
    pub outcomes: Vec<Result<RgbDenoiseReceipt, String>>,
}

impl fmt::Display for ProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyIdentity => "color profile identity is empty",
            Self::IdentityTooLong => "color profile identity is too long",
            Self::NonFiniteTransform => "color profile transform is non-finite",
            Self::MissingIccPayload => "color profile has no ICC payload",
            Self::IccPayloadTooLarge => "color profile ICC payload is too large",
        })
    }
}

impl std::error::Error for ProfileError {}

impl fmt::Display for StrengthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfRange { value } => {
                write!(formatter, "denoise strength {value} is outside 0..=100")
            }
        }
    }
}

impl std::error::Error for StrengthError {}

impl fmt::Display for ModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid RGB denoise model: {self:?}")
    }
}

impl std::error::Error for ModelError {}

impl fmt::Display for TiffRecipeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TIFF output byte limit must be nonzero")
    }
}

impl std::error::Error for TiffRecipeError {}

impl fmt::Display for RequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RGB denoise request has an invalid destination")
    }
}

impl std::error::Error for RequestError {}
