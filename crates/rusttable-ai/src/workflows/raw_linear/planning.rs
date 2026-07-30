use rusttable_color::{
    AdaptationMethod, AlphaTransform, BuiltinColorTransformPlanner, ColorEncoding, ColorRole,
    ColorTransformPlanner, ColorTransformRequest, ExtendedRange, Precision, RenderingIntent,
    TransformPlan, TransformStep,
};
use rusttable_image::{CfaPattern, ImageDimensions, Roi};
use rusttable_processing::{DemosaicAlgorithm, DemosaicPlan, RawPrepareConfig, RawPreparePlan};

use super::ports::{RawLinearModelDescriptor, selected_provider};
use super::types::{
    LinearRawRgbU16, RawLinearDenoiseRequest, RawLinearPlan, RawLinearSource, RawLinearSourceKind,
    RawLinearTile, RawOperationKind, RawOperationSpec, edit_identity,
};

#[derive(Debug, Clone, PartialEq)]
pub enum RawLinearPlanError {
    UnsupportedSource,
    XTransPatternRequired,
    MissingCalibration,
    InvalidActiveArea,
    RawPrepare(String),
    Demosaic(String),
    HighlightsUnavailable,
    UnsupportedOperation(RawOperationKind),
    ColorRequest,
    ColorPlanner,
    ColorPlan,
    ModelTask,
    ModelColorContract,
    ModelTensorContract,
    InvalidTile,
    ImageTooSmall,
    ArithmeticOverflow,
    MemoryLimit,
    ProviderUnqualified,
}

pub(crate) struct PreparedSource {
    pub dimensions: ImageDimensions,
    pub pixels: Vec<[f32; 3]>,
    pub color_plan: TransformPlan,
}

pub(crate) fn compile(
    request: &RawLinearDenoiseRequest,
    descriptor: &RawLinearModelDescriptor,
) -> Result<(RawLinearPlan, PreparedSource), RawLinearPlanError> {
    validate_model(descriptor)?;
    let provider = selected_provider(request.provider(), descriptor)
        .map_err(|_| RawLinearPlanError::ProviderUnqualified)?;
    let (prepared, required_operations) = prepare(request.source())?;
    let (included, excluded) = select_operations(request, &required_operations)?;
    let tiles = build_tiles(prepared.dimensions, descriptor)?;
    let plan = RawLinearPlan::new(
        request.source().source_identity(),
        edit_identity(request.edit()),
        descriptor.identity,
        request.source().kind(),
        prepared.dimensions,
        included,
        excluded,
        prepared.color_plan.clone(),
        tiles,
        request.strength(),
    )
    .map_err(|_| RawLinearPlanError::ColorPlan)?;
    let _ = provider;
    Ok((plan, prepared))
}

fn validate_model(descriptor: &RawLinearModelDescriptor) -> Result<(), RawLinearPlanError> {
    if descriptor.identity == [0; 32] || descriptor.task != crate::ModelTask::RawLinearDenoise {
        return Err(RawLinearPlanError::ModelTask);
    }
    if descriptor.input_color != ColorEncoding::LinearRec2020D65
        || descriptor.output_color != ColorEncoding::LinearRec2020D65
    {
        return Err(RawLinearPlanError::ModelColorContract);
    }
    if descriptor.layout != crate::TensorLayout::PlanarNchwRgb
        || !descriptor.full_range
        || !descriptor.finite_only
    {
        return Err(RawLinearPlanError::ModelTensorContract);
    }
    if descriptor.tile_width == 0
        || descriptor.tile_height == 0
        || descriptor.valid_crop.left + descriptor.valid_crop.right >= descriptor.tile_width
        || descriptor.valid_crop.top + descriptor.valid_crop.bottom >= descriptor.tile_height
        || descriptor.overlap >= descriptor.tile_width.min(descriptor.tile_height)
        || descriptor.minimum_width == 0
        || descriptor.minimum_height == 0
        || descriptor.estimated_session_bytes == 0
    {
        return Err(RawLinearPlanError::InvalidTile);
    }
    Ok(())
}

fn prepare(
    source: &RawLinearSource,
) -> Result<(PreparedSource, Vec<RawOperationKind>), RawLinearPlanError> {
    match source {
        RawLinearSource::XTrans {
            raw,
            active_area,
            calibration,
            ..
        } => {
            if !matches!(raw.pattern(), CfaPattern::XTrans(_)) {
                return Err(RawLinearPlanError::XTransPatternRequired);
            }
            let prepare = RawPreparePlan::new(raw, RawPrepareConfig::new(*active_area))
                .map_err(|error| RawLinearPlanError::RawPrepare(error.to_string()))?;
            let normalized = prepare
                .execute(raw)
                .map_err(|error| RawLinearPlanError::RawPrepare(error.to_string()))?;
            let demosaic = DemosaicPlan::new(&normalized, DemosaicAlgorithm::Bilinear)
                .map_err(|error| RawLinearPlanError::Demosaic(error.to_string()))?;
            let rgb = demosaic
                .execute(&normalized)
                .map_err(|error| RawLinearPlanError::Demosaic(error.to_string()))?
                .pixels()
                .iter()
                .map(|pixel| [pixel.red().get(), pixel.green().get(), pixel.blue().get()])
                .collect::<Vec<_>>();
            let color_plan = matrix_plan(calibration.camera_to_rec2020())?;
            Ok((
                PreparedSource {
                    dimensions: prepare.output_dimensions(),
                    pixels: rgb,
                    color_plan,
                },
                vec![RawOperationKind::RawPrepare, RawOperationKind::Demosaic],
            ))
        }
        RawLinearSource::AlreadyLinearRgb { image, calibration } => {
            let denominator = f32::from(image.white_level() - image.black_level());
            let pixels = (0..image.samples().len())
                .step_by(3)
                .map(|index| {
                    let sample = &image.samples()[index..index + 3];
                    [
                        (f32::from(sample[0]) - f32::from(image.black_level())) / denominator,
                        (f32::from(sample[1]) - f32::from(image.black_level())) / denominator,
                        (f32::from(sample[2]) - f32::from(image.black_level())) / denominator,
                    ]
                })
                .collect::<Vec<_>>();
            if pixels.iter().flatten().any(|value| !value.is_finite()) {
                return Err(RawLinearPlanError::UnsupportedSource);
            }
            let color_plan = if let Some(calibration) = calibration {
                matrix_plan(calibration.camera_to_rec2020())?
            } else {
                builtin_plan(image.color())?
            };
            Ok((
                PreparedSource {
                    dimensions: image.dimensions(),
                    pixels,
                    color_plan,
                },
                vec![RawOperationKind::RawPrepare],
            ))
        }
    }
}

fn select_operations(
    request: &RawLinearDenoiseRequest,
    required: &[RawOperationKind],
) -> Result<(Vec<RawOperationSpec>, Vec<RawOperationSpec>), RawLinearPlanError> {
    let mut included = Vec::new();
    let mut excluded = Vec::new();
    for kind in required {
        if let Some(operation) = request
            .edit()
            .operations()
            .iter()
            .find(|operation| operation.kind == *kind && operation.enabled)
        {
            included.push(operation.clone());
        } else {
            included.push(default_operation(*kind));
        }
    }
    for operation in request.edit().operations() {
        if required.contains(&operation.kind) && operation.enabled {
            continue;
        }
        if operation.enabled && operation.kind == RawOperationKind::Highlights {
            return Err(RawLinearPlanError::HighlightsUnavailable);
        }
        if operation.enabled && !required.contains(&operation.kind) {
            excluded.push(operation.clone());
        }
    }
    Ok((included, excluded))
}

fn default_operation(kind: RawOperationKind) -> RawOperationSpec {
    let id = match kind {
        RawOperationKind::RawPrepare => 0x5241_5750_5245_5041,
        RawOperationKind::Demosaic => 0x4445_4d4f_5341_4943,
        RawOperationKind::Highlights => 0x4849_4748_4c49_4748,
        _ => 0x5241_575f_4f50_4552,
    };
    RawOperationSpec {
        id,
        version: 1,
        kind,
        parameters: Vec::new(),
        enabled: true,
    }
}

fn build_tiles(
    dimensions: ImageDimensions,
    descriptor: &RawLinearModelDescriptor,
) -> Result<Vec<RawLinearTile>, RawLinearPlanError> {
    if dimensions.width() < descriptor.minimum_width
        || dimensions.height() < descriptor.minimum_height
    {
        return Err(RawLinearPlanError::ImageTooSmall);
    }
    let crop = descriptor.valid_crop;
    let core_width = descriptor
        .tile_width
        .checked_sub(crop.left + crop.right)
        .ok_or(RawLinearPlanError::ArithmeticOverflow)?;
    let core_height = descriptor
        .tile_height
        .checked_sub(crop.top + crop.bottom)
        .ok_or(RawLinearPlanError::ArithmeticOverflow)?;
    if core_width == 0 || core_height == 0 {
        return Err(RawLinearPlanError::InvalidTile);
    }
    let mut tiles = Vec::new();
    let mut y = 0;
    while y < dimensions.height() {
        let mut x = 0;
        while x < dimensions.width() {
            let input_x = x
                .saturating_sub(crop.left)
                .min(dimensions.width().saturating_sub(descriptor.tile_width));
            let input_y = y
                .saturating_sub(crop.top)
                .min(dimensions.height().saturating_sub(descriptor.tile_height));
            let output_width = core_width.min(dimensions.width() - x);
            let output_height = core_height.min(dimensions.height() - y);
            tiles.push(RawLinearTile::new(
                input_x,
                input_y,
                descriptor.tile_width,
                descriptor.tile_height,
                x,
                y,
                output_width,
                output_height,
                descriptor.overlap,
            ));
            x = x
                .checked_add(core_width)
                .ok_or(RawLinearPlanError::ArithmeticOverflow)?;
        }
        y = y
            .checked_add(core_height)
            .ok_or(RawLinearPlanError::ArithmeticOverflow)?;
    }
    Ok(tiles)
}

fn request_for(
    source: ColorEncoding,
    target: ColorEncoding,
) -> Result<ColorTransformRequest, RawLinearPlanError> {
    ColorTransformRequest::new(
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
    .map_err(|_| RawLinearPlanError::ColorRequest)
}

fn builtin_plan(source: ColorEncoding) -> Result<TransformPlan, RawLinearPlanError> {
    let request = request_for(source, ColorEncoding::LinearRec2020D65)?;
    BuiltinColorTransformPlanner
        .plan(&request)
        .map_err(|_| RawLinearPlanError::ColorPlanner)
}

fn matrix_plan(matrix: rusttable_color::Matrix3) -> Result<TransformPlan, RawLinearPlanError> {
    let request = request_for(
        ColorEncoding::LinearRec2020D65,
        ColorEncoding::LinearRec2020D65,
    )?;
    TransformPlan::new(request, vec![TransformStep::Matrix(matrix)])
        .map_err(|_| RawLinearPlanError::ColorPlan)
}

#[allow(dead_code)]
fn _active_area_dimensions(roi: Option<Roi>) -> Option<ImageDimensions> {
    roi.and_then(|roi| ImageDimensions::new(roi.width(), roi.height()).ok())
}

impl std::fmt::Display for RawLinearPlanError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "RAW-linear plan failed: {self:?}")
    }
}
impl std::error::Error for RawLinearPlanError {}

impl std::fmt::Display for super::types::PlanIdentityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("RAW-linear plan identity could not be computed")
    }
}
impl std::error::Error for super::types::PlanIdentityError {}

impl std::fmt::Display for RawLinearSourceKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::XTrans => "x-trans",
            Self::AlreadyLinearRgb => "already-linear-rgb",
        })
    }
}

impl std::fmt::Display for LinearRawRgbU16 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "LinearRawRgbU16({}x{})",
            self.dimensions().width(),
            self.dimensions().height()
        )
    }
}
