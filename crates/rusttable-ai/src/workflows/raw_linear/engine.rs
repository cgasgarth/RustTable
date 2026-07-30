use sha2::{Digest, Sha256};

use super::planning::{self, PreparedSource};
use super::ports::{
    ImportGroupingOutcome, RawLinearCatalogError, RawLinearCatalogPort, RawLinearControl,
    RawLinearDenoiseModel, RawLinearDngPublisher, RawLinearModelError, RawLinearObserver,
    RawLinearProgress, RawLinearStage, RawLinearTileInput, RawLinearWorkflowError,
};
use super::types::{
    LinearRawRgbU16, RawLinearDenoiseRequest, RawLinearPlan, RawLinearReceipt, RawLinearTile,
};
use crate::{CancellationToken, Provider, ProviderPolicy};

pub struct RawLinearDenoiseWorkflow<'a> {
    model: &'a dyn RawLinearDenoiseModel,
    publisher: &'a mut dyn RawLinearDngPublisher,
    catalog: Option<&'a mut dyn RawLinearCatalogPort>,
    observer: &'a dyn RawLinearObserver,
    control: &'a dyn RawLinearControl,
}

impl<'a> RawLinearDenoiseWorkflow<'a> {
    pub fn new(
        model: &'a dyn RawLinearDenoiseModel,
        publisher: &'a mut dyn RawLinearDngPublisher,
    ) -> Self {
        Self {
            model,
            publisher,
            catalog: None,
            observer: &super::NOOP_OBSERVER,
            control: &super::NOOP_CONTROL,
        }
    }

    #[must_use]
    pub fn with_catalog(mut self, catalog: &'a mut dyn RawLinearCatalogPort) -> Self {
        self.catalog = Some(catalog);
        self
    }

    #[must_use]
    pub fn with_observer(mut self, observer: &'a dyn RawLinearObserver) -> Self {
        self.observer = observer;
        self
    }

    #[must_use]
    pub fn with_control(mut self, control: &'a dyn RawLinearControl) -> Self {
        self.control = control;
        self
    }

    pub fn run(
        &mut self,
        request: &RawLinearDenoiseRequest,
        cancellation: &CancellationToken,
    ) -> Result<RawLinearReceipt, RawLinearWorkflowError> {
        if let Some(catalog) = self.catalog.as_deref_mut()
            && let Some(receipt) = catalog
                .reconcile(request.identity())
                .map_err(RawLinearWorkflowError::Catalog)?
        {
            return Ok(receipt);
        }
        self.check_cancel(cancellation, RawLinearStage::Validate)?;
        self.observer.progress(RawLinearProgress {
            stage: RawLinearStage::Validate,
            completed: 1,
            total: 1,
        });
        let (plan, prepared) = planning::compile(request, self.model.descriptor())
            .map_err(RawLinearWorkflowError::Plan)?;
        self.check_cancel(cancellation, RawLinearStage::Prepare)?;
        self.observer.progress(RawLinearProgress {
            stage: RawLinearStage::Prepare,
            completed: 1,
            total: 1,
        });
        let canonical_source = color_source(&plan, &prepared, cancellation, self.control)?;
        self.observer.progress(RawLinearProgress {
            stage: RawLinearStage::Color,
            completed: 1,
            total: 1,
        });

        let (canonical, provider) = if request.strength().is_zero() {
            (
                canonical_source.clone(),
                initial_provider(request.provider(), self.model.descriptor()),
            )
        } else {
            let (pixels, provider) = infer(
                request,
                &plan,
                &canonical_source,
                self.model,
                cancellation,
                self.control,
                self.observer,
            )?;
            (pixels, provider)
        };
        let output = quantize(request, &plan, &canonical, cancellation, self.control)?;
        self.observer.progress(RawLinearProgress {
            stage: RawLinearStage::Quantize,
            completed: 1,
            total: 1,
        });
        self.check_cancel(cancellation, RawLinearStage::Publish)?;
        let artifact = self
            .publisher
            .publish(request, &output, cancellation)
            .map_err(RawLinearWorkflowError::Publish)?;
        self.observer.progress(RawLinearProgress {
            stage: RawLinearStage::Publish,
            completed: 1,
            total: 1,
        });
        self.check_cancel(cancellation, RawLinearStage::Probe)?;
        let round_trip = self.publisher.probe(&artifact).map_err(|error| {
            self.publisher.discard(&artifact);
            RawLinearWorkflowError::Publish(error)
        })?;
        if round_trip != output {
            self.publisher.discard(&artifact);
            return Err(RawLinearWorkflowError::RoundTripMismatch);
        }
        self.observer.progress(RawLinearProgress {
            stage: RawLinearStage::Probe,
            completed: 1,
            total: 1,
        });

        let mut imported = false;
        let mut grouped = false;
        if request.output().add_to_catalog() {
            self.check_cancel(cancellation, RawLinearStage::Import)?;
            let catalog = self.catalog.as_deref_mut().ok_or_else(|| {
                self.publisher.discard(&artifact);
                RawLinearWorkflowError::Catalog(RawLinearCatalogError::Unavailable)
            })?;
            match catalog
                .import_and_group(request, &artifact)
                .map_err(RawLinearWorkflowError::Catalog)?
            {
                ImportGroupingOutcome::Imported { grouped: value }
                | ImportGroupingOutcome::AlreadyPresent { grouped: value } => {
                    imported = true;
                    grouped = value;
                }
            }
            self.observer.progress(RawLinearProgress {
                stage: RawLinearStage::Import,
                completed: 1,
                total: 1,
            });
            if grouped {
                self.observer.progress(RawLinearProgress {
                    stage: RawLinearStage::Group,
                    completed: 1,
                    total: 1,
                });
            }
        }
        Ok(RawLinearReceipt {
            workflow_version: super::types::RAW_LINEAR_WORKFLOW_VERSION,
            request_identity: request.identity(),
            source_identity: plan.source_identity(),
            edit_identity: plan.edit_identity(),
            plan_identity: plan.identity(),
            model_identity: plan.model_identity(),
            provider,
            tile_count: u64::try_from(plan.tiles().len()).unwrap_or(u64::MAX),
            strength_millis: (request.strength().value() * 1000.0).round() as u16,
            output_identity: output_identity(request, &output),
            imported,
            grouped,
        })
    }

    fn check_cancel(
        &self,
        cancellation: &CancellationToken,
        stage: RawLinearStage,
    ) -> Result<(), RawLinearWorkflowError> {
        if cancellation.is_cancelled() || self.control.is_cancelled(stage) {
            Err(RawLinearWorkflowError::Cancelled(stage))
        } else {
            Ok(())
        }
    }
}

fn color_source(
    plan: &RawLinearPlan,
    prepared: &PreparedSource,
    cancellation: &CancellationToken,
    control: &dyn RawLinearControl,
) -> Result<Vec<[f32; 3]>, RawLinearWorkflowError> {
    prepared
        .pixels
        .iter()
        .enumerate()
        .map(|(index, pixel)| {
            if cancellation.is_cancelled() || control.is_cancelled(RawLinearStage::Color) {
                return Err(RawLinearWorkflowError::Cancelled(RawLinearStage::Color));
            }
            plan.color_plan()
                .apply_rgb(*pixel, || cancellation.is_cancelled())
                .map_err(|_| RawLinearWorkflowError::NonFiniteOutput)
                .and_then(|value| {
                    if value.iter().all(|component| component.is_finite()) {
                        Ok(value)
                    } else {
                        let _ = index;
                        Err(RawLinearWorkflowError::NonFiniteOutput)
                    }
                })
        })
        .collect()
}

fn infer(
    request: &RawLinearDenoiseRequest,
    plan: &RawLinearPlan,
    source: &[[f32; 3]],
    model: &dyn RawLinearDenoiseModel,
    cancellation: &CancellationToken,
    control: &dyn RawLinearControl,
    observer: &dyn RawLinearObserver,
) -> Result<(Vec<[f32; 3]>, Provider), RawLinearWorkflowError> {
    let dimensions = plan.source_dimensions();
    let width = usize::try_from(dimensions.width()).map_err(|_| {
        RawLinearWorkflowError::Plan(super::planning::RawLinearPlanError::ArithmeticOverflow)
    })?;
    let tile = model.descriptor().tile_width;
    let plane = usize::try_from(tile)
        .ok()
        .and_then(|value| value.checked_mul(usize::try_from(model.descriptor().tile_height).ok()?))
        .ok_or(RawLinearWorkflowError::Plan(
            super::planning::RawLinearPlanError::ArithmeticOverflow,
        ))?;
    let mut output = vec![[0.0; 3]; source.len()];
    let mut provider = initial_provider(request.provider(), model.descriptor());
    for (completed, tile_plan) in plan.tiles().iter().copied().enumerate() {
        if cancellation.is_cancelled() || control.is_cancelled(RawLinearStage::Inference) {
            return Err(RawLinearWorkflowError::Cancelled(RawLinearStage::Inference));
        }
        let input = fill_tile(
            source,
            dimensions,
            tile_plan,
            tile,
            plane,
            model.descriptor().valid_crop,
        )?;
        let input_tensor =
            RawLinearTileInput::new(tile_plan, tile, model.descriptor().tile_height, &input);
        let inferred = model.infer(provider, &input_tensor, cancellation);
        let tile_output = match inferred {
            Err(_) if request.provider() == ProviderPolicy::Auto && provider != Provider::Cpu => {
                provider = Provider::Cpu;
                model
                    .infer(provider, &input_tensor, cancellation)
                    .map_err(RawLinearWorkflowError::Model)?
            }
            Ok(value) => value,
            Err(error) => return Err(RawLinearWorkflowError::Model(error)),
        };
        let expected = plane.checked_mul(3).ok_or(RawLinearWorkflowError::Plan(
            super::planning::RawLinearPlanError::ArithmeticOverflow,
        ))?;
        if tile_output.len() != expected || tile_output.iter().any(|value| !value.is_finite()) {
            return Err(RawLinearWorkflowError::Model(
                RawLinearModelError::InvalidOutput,
            ));
        }
        let (output_x, output_y) = tile_plan.output_origin();
        let (output_width, output_height) = tile_plan.output_dimensions();
        let crop = model.descriptor().valid_crop;
        for local_y in 0..output_height {
            for local_x in 0..output_width {
                let global_x = output_x + local_x;
                let global_y = output_y + local_y;
                let global = usize::try_from(global_y)
                    .ok()
                    .and_then(|y| y.checked_mul(width))
                    .and_then(|value| value.checked_add(usize::try_from(global_x).ok()?))
                    .ok_or(RawLinearWorkflowError::Plan(
                        super::planning::RawLinearPlanError::ArithmeticOverflow,
                    ))?;
                let tile_x = (local_x + crop.left).min(tile.saturating_sub(1));
                let tile_y =
                    (local_y + crop.top).min(model.descriptor().tile_height.saturating_sub(1));
                let tile_index = usize::try_from(tile_y)
                    .ok()
                    .and_then(|y| y.checked_mul(usize::try_from(tile).ok()?))
                    .and_then(|value| value.checked_add(usize::try_from(tile_x).ok()?))
                    .ok_or(RawLinearWorkflowError::Plan(
                        super::planning::RawLinearPlanError::ArithmeticOverflow,
                    ))?;
                output[global] = [
                    tile_output[tile_index],
                    tile_output[plane + tile_index],
                    tile_output[2 * plane + tile_index],
                ];
            }
        }
        observer.progress(RawLinearProgress {
            stage: RawLinearStage::Inference,
            completed: u64::try_from(completed + 1).unwrap_or(u64::MAX),
            total: u64::try_from(plan.tiles().len()).unwrap_or(u64::MAX),
        });
    }
    let strength = request.strength().value();
    for (source, output) in source.iter().zip(&mut output) {
        for channel in 0..3 {
            output[channel] = source[channel] + strength * (output[channel] - source[channel]);
        }
    }
    Ok((output, provider))
}

fn fill_tile(
    source: &[[f32; 3]],
    dimensions: rusttable_image::ImageDimensions,
    tile: RawLinearTile,
    tile_width: u32,
    plane: usize,
    crop: crate::TileCrop,
) -> Result<Vec<f32>, RawLinearWorkflowError> {
    let width = usize::try_from(dimensions.width()).map_err(|_| {
        RawLinearWorkflowError::Plan(super::planning::RawLinearPlanError::ArithmeticOverflow)
    })?;
    let height = dimensions.height();
    let tile_height = tile.input_dimensions().1;
    let (origin_x, origin_y) = tile.output_origin();
    let mut output = vec![
        0.0;
        plane.checked_mul(3).ok_or(RawLinearWorkflowError::Plan(
            super::planning::RawLinearPlanError::ArithmeticOverflow,
        ))?
    ];
    for y in 0..tile_height {
        for x in 0..tile_width {
            let source_x = reflect(
                origin_x.saturating_add(x).saturating_sub(crop.left),
                dimensions.width(),
            );
            let source_y = reflect(origin_y.saturating_add(y).saturating_sub(crop.top), height);
            let source_index = usize::try_from(source_y)
                .ok()
                .and_then(|value| value.checked_mul(width))
                .and_then(|value| value.checked_add(usize::try_from(source_x).ok()?))
                .ok_or(RawLinearWorkflowError::Plan(
                    super::planning::RawLinearPlanError::ArithmeticOverflow,
                ))?;
            let tile_index = usize::try_from(y)
                .ok()
                .and_then(|value| value.checked_mul(usize::try_from(tile_width).ok()?))
                .and_then(|value| value.checked_add(usize::try_from(x).ok()?))
                .ok_or(RawLinearWorkflowError::Plan(
                    super::planning::RawLinearPlanError::ArithmeticOverflow,
                ))?;
            for channel in 0..3 {
                output[channel * plane + tile_index] = source[source_index][channel];
            }
        }
    }
    Ok(output)
}

fn reflect(value: u32, limit: u32) -> u32 {
    if limit <= 1 {
        return 0;
    }
    let period = 2 * limit - 2;
    let value = value % period;
    if value < limit { value } else { period - value }
}

fn initial_provider(
    policy: ProviderPolicy,
    descriptor: &super::ports::RawLinearModelDescriptor,
) -> Provider {
    match policy {
        ProviderPolicy::Cpu => Provider::Cpu,
        ProviderPolicy::Explicit(provider) => provider,
        ProviderPolicy::Auto => descriptor
            .qualified_providers
            .iter()
            .copied()
            .find(|provider| *provider != Provider::Cpu)
            .unwrap_or(Provider::Cpu),
    }
}

fn quantize(
    request: &RawLinearDenoiseRequest,
    plan: &RawLinearPlan,
    processed: &[[f32; 3]],
    cancellation: &CancellationToken,
    control: &dyn RawLinearControl,
) -> Result<LinearRawRgbU16, RawLinearWorkflowError> {
    let white = f64::from(request.output().quantization_white());
    let mut samples = Vec::with_capacity(processed.len().saturating_mul(3));
    for pixel in processed {
        if cancellation.is_cancelled() || control.is_cancelled(RawLinearStage::Quantize) {
            return Err(RawLinearWorkflowError::Cancelled(RawLinearStage::Quantize));
        }
        for value in pixel {
            if !value.is_finite() || *value < 0.0 || *value > 1.0 {
                return Err(RawLinearWorkflowError::QuantizationRange);
            }
            samples.push(round_ties_even(f64::from(*value) * white) as u16);
        }
    }
    LinearRawRgbU16::new(
        plan.source_dimensions(),
        samples,
        0,
        request.output().quantization_white(),
        rusttable_color::ColorEncoding::LinearRec2020D65,
        rusttable_image::Orientation::Normal,
        request.output().camera_identity(),
    )
    .map_err(|_| RawLinearWorkflowError::QuantizationRange)
}

fn round_ties_even(value: f64) -> f64 {
    let floor = value.floor();
    let fraction = value - floor;
    if fraction < 0.5
        || (fraction.to_bits() == 0.5_f64.to_bits() && (floor as u64).is_multiple_of(2))
    {
        floor
    } else {
        floor + 1.0
    }
}

#[allow(dead_code)]
fn output_identity(request: &RawLinearDenoiseRequest, output: &LinearRawRgbU16) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(request.identity());
    for sample in output.samples() {
        hasher.update(sample.to_le_bytes());
    }
    hasher.finalize().into()
}
