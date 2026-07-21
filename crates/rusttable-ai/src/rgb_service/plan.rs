use std::fmt;
use std::sync::Arc;

use rusttable_color::{
    AdaptationMethod, AlphaTransform, BuiltinColorTransformPlanner, ColorEncoding, ColorRole,
    ColorTransformPlanner, ColorTransformRequest, ExtendedRange, Precision, RenderingIntent,
    TransformPlan,
};
use sha2::{Digest, Sha256};

use crate::model::{
    AlphaPolicy, EdgePadding, ModelColorContract, ModelManifest, ModelTask, Provider,
    ProviderPolicy, TensorLayout, TransferFunction,
};
use crate::{CancellationToken, ImageDimensions};

use super::detail::{DetailRecoveryPlan, DetailRecoverySettings};
use super::image::RgbAiImage;
use super::policy::{
    GamutPolicy, MemoryBudget, MemoryReceipt, PolicyError, ShadowDecision, TileReceipt,
    selected_provider,
};
use super::receipt::RgbColorReceipt;
use super::tensor::{AuxiliaryTensorSpec, RgbAiTileInput, RgbAuxiliaryTensor};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RgbAiTileSpec {
    width: u32,
    height: u32,
    overlap: u32,
    valid_crop: crate::TileCrop,
    scale: u32,
    edge_padding: EdgePadding,
}

impl RgbAiTileSpec {
    pub const fn new(
        width: u32,
        height: u32,
        overlap: u32,
        valid_crop: crate::TileCrop,
        scale: u32,
    ) -> Result<Self, RgbAiManifestError> {
        if width == 0
            || height == 0
            || !matches!(scale, 1 | 2 | 4)
            || valid_crop.left.saturating_add(valid_crop.right) >= width
            || valid_crop.top.saturating_add(valid_crop.bottom) >= height
            || overlap >= if width < height { width } else { height }
        {
            return Err(RgbAiManifestError::InvalidTile);
        }
        Ok(Self {
            width,
            height,
            overlap,
            valid_crop,
            scale,
            edge_padding: EdgePadding::Mirror,
        })
    }

    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }
    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }
    #[must_use]
    pub const fn overlap(self) -> u32 {
        self.overlap
    }
    #[must_use]
    pub const fn valid_crop(self) -> crate::TileCrop {
        self.valid_crop
    }
    #[must_use]
    pub const fn scale(self) -> u32 {
        self.scale
    }
    #[must_use]
    pub const fn edge_padding(self) -> EdgePadding {
        self.edge_padding
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RgbAiManifest {
    id: String,
    version: String,
    task: ModelTask,
    model_hash: [u8; 32],
    model_space: ColorEncoding,
    color: ModelColorContract,
    tile: RgbAiTileSpec,
    alpha: AlphaPolicy,
    shadow_boost: bool,
    auxiliary: Vec<AuxiliaryTensorSpec>,
    providers: Vec<Provider>,
    estimated_session_bytes: u64,
    estimated_model_output_bytes: u64,
}

impl RgbAiManifest {
    pub fn new(
        id: impl Into<String>,
        version: impl Into<String>,
        task: ModelTask,
        model_hash: [u8; 32],
        tile: RgbAiTileSpec,
    ) -> Result<Self, RgbAiManifestError> {
        let id = id.into();
        let version = version.into();
        if id.is_empty() || id.len() > 128 || version.is_empty() || version.len() > 64 {
            return Err(RgbAiManifestError::InvalidIdentity);
        }
        if !matches!(
            task,
            ModelTask::RgbDenoise | ModelTask::SuperResolution2x | ModelTask::SuperResolution4x
        ) {
            return Err(RgbAiManifestError::UnsupportedTask);
        }
        Ok(Self {
            id,
            version,
            task,
            model_hash,
            model_space: ColorEncoding::SrgbD65,
            color: ModelColorContract::default(),
            tile,
            alpha: AlphaPolicy::PreserveNearest,
            shadow_boost: false,
            auxiliary: Vec::new(),
            providers: vec![Provider::Cpu],
            estimated_session_bytes: 1,
            estimated_model_output_bytes: 0,
        })
    }

    pub fn from_model_manifest(manifest: &ModelManifest) -> Result<Self, RgbAiManifestError> {
        if manifest.color() != ModelColorContract::default() {
            return Err(RgbAiManifestError::UnsupportedColorContract);
        }
        let tile = manifest.tile();
        let tile = RgbAiTileSpec::new(
            tile.width,
            tile.height,
            tile.overlap,
            tile.valid_crop,
            tile.scale,
        )?;
        let mut hash = [0_u8; 32];
        hash.copy_from_slice(&Sha256::digest(manifest.identity().as_bytes()));
        let auxiliary = manifest
            .auxiliary()
            .iter()
            .map(|tensor| match tensor {
                crate::AuxiliaryTensor::Constant {
                    name,
                    channels,
                    value,
                } => AuxiliaryTensorSpec::constant(name, *channels, *value)
                    .map_err(|_| RgbAiManifestError::InvalidAuxiliary),
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            id: manifest.id().to_owned(),
            version: manifest.version().to_owned(),
            task: manifest.task(),
            model_hash: hash,
            model_space: ColorEncoding::SrgbD65,
            color: manifest.color(),
            tile,
            alpha: manifest.alpha(),
            shadow_boost: manifest.shadow_boost(),
            auxiliary,
            providers: manifest.providers().to_vec(),
            estimated_session_bytes: 1,
            estimated_model_output_bytes: 0,
        })
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }
    #[must_use]
    pub const fn task(&self) -> ModelTask {
        self.task
    }
    #[must_use]
    pub const fn model_hash(&self) -> [u8; 32] {
        self.model_hash
    }
    #[must_use]
    pub const fn model_space(&self) -> ColorEncoding {
        self.model_space
    }
    #[must_use]
    pub const fn tile(&self) -> RgbAiTileSpec {
        self.tile
    }
    #[must_use]
    pub const fn alpha(&self) -> AlphaPolicy {
        self.alpha
    }
    #[must_use]
    pub const fn shadow_boost(&self) -> bool {
        self.shadow_boost
    }
    #[must_use]
    pub fn auxiliary(&self) -> &[AuxiliaryTensorSpec] {
        &self.auxiliary
    }
    #[must_use]
    pub fn providers(&self) -> &[Provider] {
        &self.providers
    }

    #[must_use]
    pub fn with_auxiliary(mut self, auxiliary: Vec<AuxiliaryTensorSpec>) -> Self {
        self.auxiliary = auxiliary;
        self
    }

    #[must_use]
    pub const fn with_shadow_boost(mut self, enabled: bool) -> Self {
        self.shadow_boost = enabled;
        self
    }

    #[must_use]
    pub fn with_providers(mut self, providers: Vec<Provider>) -> Self {
        self.providers = providers;
        self
    }

    #[must_use]
    pub const fn with_alpha_policy(mut self, alpha: AlphaPolicy) -> Self {
        self.alpha = alpha;
        self
    }

    #[must_use]
    pub const fn with_resource_estimates(
        mut self,
        session_bytes: u64,
        model_output_bytes: u64,
    ) -> Self {
        self.estimated_session_bytes = session_bytes;
        self.estimated_model_output_bytes = model_output_bytes;
        self
    }

    pub fn validate(&self) -> Result<(), RgbAiManifestError> {
        if self.providers.is_empty() || !self.providers.contains(&Provider::Cpu) {
            return Err(RgbAiManifestError::CpuProviderRequired);
        }
        if self.color != ModelColorContract::default()
            || self.model_space != ColorEncoding::SrgbD65
            || self.color.input_transfer != TransferFunction::ExtendedSrgb
            || self.color.output_transfer != TransferFunction::ExtendedSrgb
            || self.color.layout != TensorLayout::PlanarNchwRgb
        {
            return Err(RgbAiManifestError::UnsupportedColorContract);
        }
        if self.auxiliary.iter().any(|tensor| tensor.name().is_empty()) {
            return Err(RgbAiManifestError::InvalidAuxiliary);
        }
        if self.tile.scale() != self.task.scale_factor() {
            return Err(RgbAiManifestError::ScaleMismatch);
        }
        Ok(())
    }

    pub(crate) const fn estimated_session_bytes(&self) -> u64 {
        self.estimated_session_bytes
    }
    pub(crate) const fn estimated_model_output_bytes(&self) -> u64 {
        self.estimated_model_output_bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RgbAiManifestError {
    InvalidIdentity,
    UnsupportedTask,
    InvalidTile,
    UnsupportedColorContract,
    InvalidAuxiliary,
    CpuProviderRequired,
    ScaleMismatch,
}

impl fmt::Display for RgbAiManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid RGB AI manifest: {self:?}")
    }
}
impl std::error::Error for RgbAiManifestError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RgbAiProfileError {
    UnsupportedProfile,
    Transform(rusttable_color::ColorTransformRequestError),
    Planner(rusttable_color::PlannerError),
}

impl fmt::Display for RgbAiProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "RGB AI profile transform failed: {self:?}")
    }
}
impl std::error::Error for RgbAiProfileError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RgbAiTile {
    input_x: i64,
    input_y: i64,
    input_width: u32,
    input_height: u32,
    output_x: u32,
    output_y: u32,
    output_width: u32,
    output_height: u32,
}

impl RgbAiTile {
    #[must_use]
    pub const fn input_origin(self) -> (i64, i64) {
        (self.input_x, self.input_y)
    }
    #[must_use]
    pub const fn input_dimensions(self) -> (u32, u32) {
        (self.input_width, self.input_height)
    }
    #[must_use]
    pub const fn output_origin(self) -> (u32, u32) {
        (self.output_x, self.output_y)
    }
    #[must_use]
    pub const fn output_dimensions(self) -> (u32, u32) {
        (self.output_width, self.output_height)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RgbAiOptions {
    pub gamut: GamutPolicy,
    pub shadow: crate::ShadowPolicy,
    pub detail: Option<DetailRecoverySettings>,
    pub provider: ProviderPolicy,
    pub memory: MemoryBudget,
}

impl Default for RgbAiOptions {
    fn default() -> Self {
        Self {
            gamut: GamutPolicy::Disabled,
            shadow: crate::ShadowPolicy::Disabled,
            detail: None,
            provider: ProviderPolicy::Auto,
            memory: MemoryBudget::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RgbAiPlan {
    source_identity: [u8; 32],
    source_dimensions: ImageDimensions,
    output_dimensions: ImageDimensions,
    source_profile: ColorEncoding,
    output_profile: ColorEncoding,
    model_hash: [u8; 32],
    task: ModelTask,
    scale: u32,
    input_transform: TransformPlan,
    output_transform: TransformPlan,
    tiles: Arc<[RgbAiTile]>,
    gamut_mask: Arc<[bool]>,
    gamut: GamutPolicy,
    shadow: ShadowDecision,
    detail: Option<DetailRecoveryPlan>,
    provider_policy: ProviderPolicy,
    initial_provider: Provider,
    auxiliary: Arc<[AuxiliaryTensorSpec]>,
    valid_crop: crate::TileCrop,
    alpha: AlphaPolicy,
    memory: MemoryReceipt,
    tiles_receipt: TileReceipt,
}

impl RgbAiPlan {
    pub fn build(
        source: &RgbAiImage,
        output_profile: ColorEncoding,
        manifest: &RgbAiManifest,
        options: RgbAiOptions,
    ) -> Result<Self, RgbAiPlanError> {
        manifest.validate().map_err(RgbAiPlanError::Manifest)?;
        options.gamut.validate().map_err(RgbAiPlanError::Policy)?;
        if options.gamut.enabled() && manifest.tile.scale() != 1 {
            return Err(RgbAiPlanError::Policy(PolicyError::UnsupportedScaleGamut));
        }
        if !output_profile.is_linear() {
            return Err(RgbAiPlanError::UnsupportedProfile);
        }
        let input_transform = build_transform(source.profile(), ColorEncoding::LinearSrgbD65)?;
        let output_transform = build_transform(ColorEncoding::LinearSrgbD65, output_profile)?;
        let mut converted = Vec::with_capacity(source.pixels().len());
        for pixel in source.pixels() {
            converted.push(
                input_transform
                    .apply_rgb([pixel[0], pixel[1], pixel[2]], || false)
                    .map_err(|_| RgbAiPlanError::NonFiniteTransform)?,
            );
        }
        let gamut_mask = if options.gamut.enabled() {
            converted
                .iter()
                .map(|rgb| {
                    rgb.iter().all(|value| {
                        (-options.gamut.margin()..=1.0 + options.gamut.margin()).contains(value)
                    })
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let shadow = shadow_decision(
            &converted,
            source.dimensions(),
            manifest.shadow_boost(),
            options.shadow,
        )?;
        let tiles = build_tiles(source.dimensions(), manifest.tile())?;
        let output_dimensions = ImageDimensions::new(
            source
                .dimensions()
                .width()
                .checked_mul(manifest.tile.scale())
                .ok_or(RgbAiPlanError::ArithmeticOverflow)?,
            source
                .dimensions()
                .height()
                .checked_mul(manifest.tile.scale())
                .ok_or(RgbAiPlanError::ArithmeticOverflow)?,
        )
        .map_err(|_| RgbAiPlanError::ArithmeticOverflow)?;
        if options.detail.is_some() && manifest.tile.scale() != 1 {
            return Err(RgbAiPlanError::Policy(PolicyError::UnsupportedScaleDetail));
        }
        let detail = options.detail.map(DetailRecoveryPlan::from_settings);
        let memory = memory_receipt(
            source,
            manifest,
            output_dimensions,
            options.memory,
            detail.is_some(),
            u64::try_from(gamut_mask.len()).map_err(|_| RgbAiPlanError::ArithmeticOverflow)?,
        )?;
        let initial_provider = selected_provider(options.provider, manifest.providers());
        if !manifest.providers().contains(&initial_provider) {
            return Err(RgbAiPlanError::Policy(PolicyError::ProviderUnavailable));
        }
        let tiles_receipt = TileReceipt {
            count: u64::try_from(tiles.len()).map_err(|_| RgbAiPlanError::ArithmeticOverflow)?,
            tile_width: manifest.tile.width(),
            tile_height: manifest.tile.height(),
            overlap: manifest.tile.overlap(),
            scale: manifest.tile.scale(),
        };
        Ok(Self {
            source_identity: source.identity(),
            source_dimensions: source.dimensions(),
            output_dimensions,
            source_profile: source.profile(),
            output_profile,
            model_hash: manifest.model_hash(),
            task: manifest.task(),
            scale: manifest.tile.scale(),
            input_transform,
            output_transform,
            tiles: tiles.into(),
            gamut_mask: gamut_mask.into(),
            gamut: options.gamut,
            shadow,
            detail,
            provider_policy: options.provider,
            initial_provider,
            auxiliary: manifest.auxiliary().to_vec().into(),
            valid_crop: manifest.tile.valid_crop(),
            alpha: manifest.alpha(),
            memory,
            tiles_receipt,
        })
    }

    #[must_use]
    pub const fn source_dimensions(&self) -> ImageDimensions {
        self.source_dimensions
    }
    #[must_use]
    pub const fn output_dimensions(&self) -> ImageDimensions {
        self.output_dimensions
    }
    #[must_use]
    pub const fn source_profile(&self) -> ColorEncoding {
        self.source_profile
    }
    #[must_use]
    pub const fn output_profile(&self) -> ColorEncoding {
        self.output_profile
    }
    #[must_use]
    pub const fn task(&self) -> ModelTask {
        self.task
    }
    #[must_use]
    pub const fn scale(&self) -> u32 {
        self.scale
    }
    #[must_use]
    pub const fn model_hash(&self) -> [u8; 32] {
        self.model_hash
    }
    #[must_use]
    pub const fn source_identity(&self) -> [u8; 32] {
        self.source_identity
    }
    #[must_use]
    pub fn tiles(&self) -> &[RgbAiTile] {
        &self.tiles
    }
    #[must_use]
    pub fn gamut_mask(&self) -> &[bool] {
        &self.gamut_mask
    }
    #[must_use]
    pub const fn gamut(&self) -> GamutPolicy {
        self.gamut
    }
    #[must_use]
    pub const fn shadow(&self) -> ShadowDecision {
        self.shadow
    }
    #[must_use]
    pub const fn detail(&self) -> Option<DetailRecoveryPlan> {
        self.detail
    }
    #[must_use]
    pub const fn provider_policy(&self) -> ProviderPolicy {
        self.provider_policy
    }
    #[must_use]
    pub const fn initial_provider(&self) -> Provider {
        self.initial_provider
    }
    #[must_use]
    pub fn auxiliary(&self) -> &[AuxiliaryTensorSpec] {
        &self.auxiliary
    }
    #[must_use]
    pub const fn valid_crop(&self) -> crate::TileCrop {
        self.valid_crop
    }
    #[must_use]
    pub const fn alpha(&self) -> AlphaPolicy {
        self.alpha
    }
    #[must_use]
    pub const fn memory(&self) -> MemoryReceipt {
        self.memory
    }
    #[must_use]
    pub const fn tiles_receipt(&self) -> TileReceipt {
        self.tiles_receipt
    }

    pub(crate) fn output_transform(&self) -> &TransformPlan {
        &self.output_transform
    }
    pub(crate) fn input_transform(&self) -> &TransformPlan {
        &self.input_transform
    }
    pub(crate) fn plan_hash(&self) -> Result<[u8; 32], RgbAiPlanError> {
        let mut hasher = Sha256::new();
        hasher.update(b"rusttable-ai-rgb-plan-v1");
        hasher.update(self.source_identity);
        hasher.update(self.model_hash);
        hasher.update(
            self.input_transform
                .identity()
                .map_err(RgbAiPlanError::ColorPlan)?,
        );
        hasher.update(
            self.output_transform
                .identity()
                .map_err(RgbAiPlanError::ColorPlan)?,
        );
        for tile in self.tiles.iter() {
            hasher.update(tile.input_x.to_le_bytes());
            hasher.update(tile.input_y.to_le_bytes());
            hasher.update(tile.output_x.to_le_bytes());
            hasher.update(tile.output_y.to_le_bytes());
        }
        Ok(hasher.finalize().into())
    }

    pub(crate) fn make_tile_input(
        &self,
        source: &RgbAiImage,
        tile: RgbAiTile,
        cancelled: &CancellationToken,
    ) -> Result<RgbAiTileInput, RgbAiPlanError> {
        let plane = usize::try_from(tile.input_width)
            .ok()
            .and_then(|width| {
                usize::try_from(tile.input_height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .ok_or(RgbAiPlanError::ArithmeticOverflow)?;
        let mut nchw = vec![
            0.0;
            plane
                .checked_mul(3)
                .ok_or(RgbAiPlanError::ArithmeticOverflow)?
        ];
        for local_y in 0..tile.input_height {
            for local_x in 0..tile.input_width {
                if cancelled.is_cancelled() {
                    return Err(RgbAiPlanError::Cancelled);
                }
                let source_x = super::detail::reflect(
                    tile.input_x + i64::from(local_x),
                    source.dimensions().width() as usize,
                );
                let source_y = super::detail::reflect(
                    tile.input_y + i64::from(local_y),
                    source.dimensions().height() as usize,
                );
                let source_pixel =
                    source.pixels()[source_y * source.dimensions().width() as usize + source_x];
                let mut rgb = self
                    .input_transform
                    .apply_rgb([source_pixel[0], source_pixel[1], source_pixel[2]], || {
                        cancelled.is_cancelled()
                    })
                    .map_err(|_| RgbAiPlanError::NonFiniteTransform)?;
                for channel in &mut rgb {
                    *channel = channel.clamp(0.0, 1.0);
                    if self.shadow.enabled() {
                        *channel = channel.sqrt();
                    }
                    *channel = extended_srgb_encode(*channel)
                        .map_err(|_| RgbAiPlanError::NonFiniteTransform)?;
                }
                let index = usize::try_from(local_y).unwrap()
                    * usize::try_from(tile.input_width).unwrap()
                    + usize::try_from(local_x).unwrap();
                nchw[index] = rgb[0];
                nchw[plane + index] = rgb[1];
                nchw[2 * plane + index] = rgb[2];
            }
        }
        let auxiliary = self
            .auxiliary
            .iter()
            .map(|spec| RgbAuxiliaryTensor::constant(spec, tile.input_width, tile.input_height))
            .collect();
        RgbAiTileInput::new(tile, tile.input_width, tile.input_height, nchw, auxiliary)
            .map_err(|_| RgbAiPlanError::ArithmeticOverflow)
    }
}

fn build_transform(
    source: ColorEncoding,
    target: ColorEncoding,
) -> Result<TransformPlan, RgbAiPlanError> {
    if !source.is_explicit() || !target.is_explicit() || !source.is_linear() || !target.is_linear()
    {
        return Err(RgbAiPlanError::UnsupportedProfile);
    }
    let request = ColorTransformRequest::new(
        source,
        target,
        ColorRole::Working,
        RenderingIntent::Relative,
        rusttable_color::BlackPointCompensation::Disabled,
        AdaptationMethod::Bradford,
        Precision::F32,
        AlphaTransform::Preserve,
        ExtendedRange::Extended,
        1,
    )
    .map_err(RgbAiPlanError::ColorRequest)?;
    BuiltinColorTransformPlanner
        .plan(&request)
        .map_err(RgbAiPlanError::Planner)
}

fn build_tiles(
    dimensions: ImageDimensions,
    spec: RgbAiTileSpec,
) -> Result<Vec<RgbAiTile>, RgbAiPlanError> {
    let core_width = spec
        .width()
        .checked_sub(
            spec.valid_crop()
                .left
                .checked_add(spec.valid_crop().right)
                .ok_or(RgbAiPlanError::ArithmeticOverflow)?,
        )
        .ok_or(RgbAiPlanError::InvalidTile)?;
    let core_height = spec
        .height()
        .checked_sub(
            spec.valid_crop()
                .top
                .checked_add(spec.valid_crop().bottom)
                .ok_or(RgbAiPlanError::ArithmeticOverflow)?,
        )
        .ok_or(RgbAiPlanError::InvalidTile)?;
    let mut tiles = Vec::new();
    let mut y = 0_u32;
    while y < dimensions.height() {
        let mut x = 0_u32;
        while x < dimensions.width() {
            let valid_width = core_width.min(dimensions.width() - x);
            let valid_height = core_height.min(dimensions.height() - y);
            tiles.push(RgbAiTile {
                input_x: i64::from(x) - i64::from(spec.valid_crop().left),
                input_y: i64::from(y) - i64::from(spec.valid_crop().top),
                input_width: spec.width(),
                input_height: spec.height(),
                output_x: x
                    .checked_mul(spec.scale())
                    .ok_or(RgbAiPlanError::ArithmeticOverflow)?,
                output_y: y
                    .checked_mul(spec.scale())
                    .ok_or(RgbAiPlanError::ArithmeticOverflow)?,
                output_width: valid_width
                    .checked_mul(spec.scale())
                    .ok_or(RgbAiPlanError::ArithmeticOverflow)?,
                output_height: valid_height
                    .checked_mul(spec.scale())
                    .ok_or(RgbAiPlanError::ArithmeticOverflow)?,
            });
            x = x
                .checked_add(core_width)
                .ok_or(RgbAiPlanError::ArithmeticOverflow)?;
        }
        y = y
            .checked_add(core_height)
            .ok_or(RgbAiPlanError::ArithmeticOverflow)?;
    }
    Ok(tiles)
}

fn shadow_decision(
    converted: &[[f32; 3]],
    dimensions: ImageDimensions,
    capable: bool,
    policy: crate::ShadowPolicy,
) -> Result<ShadowDecision, RgbAiPlanError> {
    let width =
        usize::try_from(dimensions.width()).map_err(|_| RgbAiPlanError::ArithmeticOverflow)?;
    let height =
        usize::try_from(dimensions.height()).map_err(|_| RgbAiPlanError::ArithmeticOverflow)?;
    let sampled = height
        .checked_add(15)
        .and_then(|value| value.checked_div(16))
        .and_then(|rows| {
            width
                .checked_add(15)
                .and_then(|value| value.checked_div(16))
                .and_then(|columns| rows.checked_mul(columns))
        })
        .and_then(|value| u64::try_from(value).ok())
        .ok_or(RgbAiPlanError::ArithmeticOverflow)?;
    if !capable || matches!(policy, crate::ShadowPolicy::Disabled) {
        return Ok(ShadowDecision::new(false, sampled, 0));
    }
    if sampled == 0 {
        return Err(RgbAiPlanError::Policy(PolicyError::InvalidShadowThreshold));
    }
    let dark = (0..height)
        .step_by(16)
        .flat_map(|y| {
            (0..width)
                .step_by(16)
                .map(move |x| converted[y * width + x])
        })
        .filter(|rgb| {
            let rgb = rgb.map(|value| value.clamp(0.0, 1.0));
            0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2] < 0.005
        })
        .count() as u64;
    Ok(ShadowDecision::new(dark * 10 >= sampled, sampled, dark))
}

fn memory_receipt(
    source: &RgbAiImage,
    manifest: &RgbAiManifest,
    output: ImageDimensions,
    budget: MemoryBudget,
    detail: bool,
    mask_bytes: u64,
) -> Result<MemoryReceipt, RgbAiPlanError> {
    let source_pixels = u64::try_from(
        source
            .dimensions()
            .pixels()
            .ok_or(RgbAiPlanError::ArithmeticOverflow)?,
    )
    .map_err(|_| RgbAiPlanError::ArithmeticOverflow)?;
    let source_bytes = bytes_for(source_pixels, 16)?;
    let tile_pixels = u64::from(manifest.tile.width())
        .checked_mul(u64::from(manifest.tile.height()))
        .ok_or(RgbAiPlanError::ArithmeticOverflow)?;
    let tile_input_bytes = bytes_for(tile_pixels, 3 * 4)?;
    let output_tile_pixels = tile_pixels
        .checked_mul(u64::from(manifest.tile.scale()).pow(2))
        .ok_or(RgbAiPlanError::ArithmeticOverflow)?;
    let tile_output_bytes = bytes_for(output_tile_pixels, 3 * 4)?;
    let row_buffer_bytes = bytes_for(u64::from(output.width()), 16)?;
    let model_output_bytes = manifest
        .estimated_model_output_bytes()
        .max(tile_output_bytes);
    let detail_bytes = if detail {
        let output_pixels =
            u64::try_from(output.pixels().ok_or(RgbAiPlanError::ArithmeticOverflow)?)
                .map_err(|_| RgbAiPlanError::ArithmeticOverflow)?;
        bytes_for(output_pixels, 4)?
    } else {
        0
    };
    MemoryReceipt::checked(
        source_bytes,
        mask_bytes,
        tile_input_bytes,
        tile_output_bytes,
        row_buffer_bytes,
        model_output_bytes,
        detail_bytes,
        manifest.estimated_session_bytes(),
        budget,
    )
    .map_err(RgbAiPlanError::Policy)
}

fn bytes_for(pixels: u64, bytes_per_pixel: u64) -> Result<u64, RgbAiPlanError> {
    pixels
        .checked_mul(bytes_per_pixel)
        .ok_or(RgbAiPlanError::ArithmeticOverflow)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtendedSrgbError {
    NonFinite,
    Overflow,
}

impl fmt::Display for ExtendedSrgbError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "extended sRGB transfer failed: {self:?}")
    }
}

impl std::error::Error for ExtendedSrgbError {}

pub fn extended_srgb_encode(value: f32) -> Result<f32, ExtendedSrgbError> {
    if !value.is_finite() {
        return Err(ExtendedSrgbError::NonFinite);
    }
    let sign = value.signum();
    let magnitude = value.abs();
    let encoded = if magnitude <= 0.003_130_8 {
        12.92 * magnitude
    } else {
        1.055 * magnitude.powf(1.0 / 2.4) - 0.055
    };
    let result = sign * encoded;
    result
        .is_finite()
        .then_some(result)
        .ok_or(ExtendedSrgbError::Overflow)
}

pub fn extended_srgb_decode(value: f32) -> Result<f32, ExtendedSrgbError> {
    if !value.is_finite() {
        return Err(ExtendedSrgbError::NonFinite);
    }
    let sign = value.signum();
    let magnitude = value.abs();
    let decoded = if magnitude <= 0.040_45 {
        magnitude / 12.92
    } else {
        ((magnitude + 0.055) / 1.055).powf(2.4)
    };
    let result = sign * decoded;
    result
        .is_finite()
        .then_some(result)
        .ok_or(ExtendedSrgbError::Overflow)
}

#[derive(Debug, Clone, PartialEq)]
pub enum RgbAiPlanError {
    Manifest(RgbAiManifestError),
    Policy(PolicyError),
    UnsupportedProfile,
    ColorRequest(rusttable_color::ColorTransformRequestError),
    Planner(rusttable_color::PlannerError),
    ColorPlan(rusttable_color::TransformPlanError),
    NonFiniteTransform,
    InvalidTile,
    ArithmeticOverflow,
    Cancelled,
}

impl fmt::Display for RgbAiPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid RGB AI plan: {self:?}")
    }
}
impl std::error::Error for RgbAiPlanError {}

impl RgbAiPlan {
    pub(crate) fn color_receipt(&self) -> Result<RgbColorReceipt, RgbAiPlanError> {
        Ok(RgbColorReceipt {
            input_plan_hash: self
                .input_transform
                .identity()
                .map_err(RgbAiPlanError::ColorPlan)?,
            output_plan_hash: self
                .output_transform
                .identity()
                .map_err(RgbAiPlanError::ColorPlan)?,
            source: self.source_profile,
            model: ColorEncoding::LinearSrgbD65,
            output: self.output_profile,
        })
    }
}
