use super::planning;
use super::ports::{
    ImportGroupingOutcome, RawBayerCatalogPort, RawBayerControl, RawBayerDenoiseModel,
    RawBayerModelError, RawBayerObserver, RawBayerProgress, RawBayerStage, RawBayerTileInput,
    RawBayerWorkflowError,
};
use super::ports::{RawBayerDngPublisher, RawBayerModelDescriptor};
use super::types::{
    CfaBayerU16, RawBayerDenoiseRequest, RawBayerPlan, RawBayerReceipt, RawBayerTile, RawFrame,
    plane_for, plane_offsets,
};
use crate::{CancellationToken, Provider, ProviderPolicy};

pub struct RawBayerDenoiseWorkflow<'a> {
    model: &'a dyn RawBayerDenoiseModel,
    publisher: &'a mut dyn RawBayerDngPublisher,
    catalog: Option<&'a mut dyn RawBayerCatalogPort>,
    observer: &'a dyn RawBayerObserver,
    control: &'a dyn RawBayerControl,
}

impl<'a> RawBayerDenoiseWorkflow<'a> {
    pub fn new(
        model: &'a dyn RawBayerDenoiseModel,
        publisher: &'a mut dyn RawBayerDngPublisher,
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
    pub fn with_catalog(mut self, catalog: &'a mut dyn RawBayerCatalogPort) -> Self {
        self.catalog = Some(catalog);
        self
    }
    #[must_use]
    pub fn with_observer(mut self, observer: &'a dyn RawBayerObserver) -> Self {
        self.observer = observer;
        self
    }
    #[must_use]
    pub fn with_control(mut self, control: &'a dyn RawBayerControl) -> Self {
        self.control = control;
        self
    }

    pub fn run(
        &mut self,
        request: &RawBayerDenoiseRequest,
        cancellation: &CancellationToken,
    ) -> Result<RawBayerReceipt, RawBayerWorkflowError> {
        if let Some(catalog) = self.catalog.as_deref_mut()
            && let Some(receipt) = catalog
                .reconcile(request.identity())
                .map_err(RawBayerWorkflowError::Catalog)?
        {
            return Ok(receipt);
        }
        self.check_cancel(cancellation, RawBayerStage::Validate)?;
        self.observer.progress(RawBayerProgress {
            stage: RawBayerStage::Validate,
            completed: 1,
            total: 1,
        });
        let plan = planning::compile(request, self.model.descriptor())
            .map_err(RawBayerWorkflowError::Plan)?;
        let mut provider = selected_provider(request.provider(), self.model.descriptor())
            .map_err(RawBayerWorkflowError::Plan)?;
        self.check_cancel(cancellation, RawBayerStage::Inverse)?;
        let output = if request.strength().is_zero() {
            CfaBayerU16::new(request.source(), request.source().raw().samples().to_vec())
                .map_err(|_| RawBayerWorkflowError::QuantizationRange)?
        } else {
            let (packed, actual_provider) = execute_tiles(
                request,
                &plan,
                self.model,
                provider,
                cancellation,
                self.control,
                self.observer,
            )?;
            provider = actual_provider;
            inverse_and_quantize(
                request.source(),
                &plan,
                self.model.descriptor(),
                &packed,
                cancellation,
                self.control,
            )?
        };
        self.observer.progress(RawBayerProgress {
            stage: RawBayerStage::Inverse,
            completed: 1,
            total: 1,
        });
        self.check_cancel(cancellation, RawBayerStage::Publish)?;
        let artifact = self
            .publisher
            .publish(request, &output, cancellation)
            .map_err(RawBayerWorkflowError::Publish)?;
        self.observer.progress(RawBayerProgress {
            stage: RawBayerStage::Publish,
            completed: 1,
            total: 1,
        });
        self.check_cancel(cancellation, RawBayerStage::Probe)?;
        let round_trip = self.publisher.probe(&artifact).map_err(|error| {
            self.publisher.discard(&artifact);
            RawBayerWorkflowError::Publish(error)
        })?;
        if round_trip != output {
            self.publisher.discard(&artifact);
            return Err(RawBayerWorkflowError::RoundTripMismatch);
        }
        self.observer.progress(RawBayerProgress {
            stage: RawBayerStage::Probe,
            completed: 1,
            total: 1,
        });
        let mut imported = false;
        let mut grouped = false;
        if request.output().add_to_catalog() {
            self.check_cancel(cancellation, RawBayerStage::Import)?;
            let catalog = self
                .catalog
                .as_deref_mut()
                .ok_or(RawBayerWorkflowError::Catalog(
                    super::ports::RawBayerCatalogError::Unavailable,
                ))?;
            match catalog
                .import_and_group(request, &artifact)
                .map_err(RawBayerWorkflowError::Catalog)?
            {
                ImportGroupingOutcome::Imported { grouped: value }
                | ImportGroupingOutcome::AlreadyPresent { grouped: value } => {
                    imported = true;
                    grouped = value;
                }
            }
            self.observer.progress(RawBayerProgress {
                stage: RawBayerStage::Import,
                completed: 1,
                total: 1,
            });
            if grouped {
                self.observer.progress(RawBayerProgress {
                    stage: RawBayerStage::Group,
                    completed: 1,
                    total: 1,
                });
            }
        }
        Ok(RawBayerReceipt {
            workflow_version: super::types::RAW_BAYER_WORKFLOW_VERSION,
            request_identity: request.identity(),
            source_identity: plan.source_identity(),
            edit_identity: plan.edit_identity(),
            plan_identity: plan.identity(),
            model_identity: plan.model_identity(),
            provider,
            tile_count: u64::try_from(plan.tiles().len()).unwrap_or(u64::MAX),
            strength_millis: (request.strength().value() * 1000.0).round() as u16,
            output_identity: output.output_identity(),
            imported,
            grouped,
        })
    }

    fn check_cancel(
        &self,
        cancellation: &CancellationToken,
        stage: RawBayerStage,
    ) -> Result<(), RawBayerWorkflowError> {
        if cancellation.is_cancelled() || self.control.is_cancelled(stage) {
            Err(RawBayerWorkflowError::Cancelled(stage))
        } else {
            Ok(())
        }
    }
}

fn selected_provider(
    policy: ProviderPolicy,
    descriptor: &RawBayerModelDescriptor,
) -> Result<Provider, super::planning::RawBayerPlanError> {
    super::ports::selected_provider(policy, descriptor)
}

fn execute_tiles(
    request: &RawBayerDenoiseRequest,
    plan: &RawBayerPlan,
    model: &dyn RawBayerDenoiseModel,
    mut provider: Provider,
    cancellation: &CancellationToken,
    control: &dyn RawBayerControl,
    observer: &dyn RawBayerObserver,
) -> Result<(Vec<f32>, Provider), RawBayerWorkflowError> {
    let packed = plan.packed_dimensions();
    let plane_size = usize::try_from(packed.width())
        .ok()
        .and_then(|w| {
            usize::try_from(packed.height())
                .ok()
                .and_then(|h| w.checked_mul(h))
        })
        .ok_or(RawBayerWorkflowError::Plan(
            super::planning::RawBayerPlanError::ArithmeticOverflow,
        ))?;
    let mut accum = vec![
        0.0_f64;
        plane_size
            .checked_mul(4)
            .ok_or(RawBayerWorkflowError::Plan(
                super::planning::RawBayerPlanError::ArithmeticOverflow
            ))?
    ];
    let mut weights = vec![0.0_f64; plane_size];
    for (index, tile) in plan.tiles().iter().enumerate() {
        if cancellation.is_cancelled() || control.is_cancelled(RawBayerStage::Pack) {
            return Err(RawBayerWorkflowError::Cancelled(RawBayerStage::Pack));
        }
        let tensor = pack_tile(
            request.source(),
            plan.processing_area(),
            tile,
            model.descriptor(),
        )
        .map_err(RawBayerWorkflowError::Frame)?;
        observer.progress(RawBayerProgress {
            stage: RawBayerStage::Pack,
            completed: u64::try_from(index + 1).unwrap_or(u64::MAX),
            total: u64::try_from(plan.tiles().len()).unwrap_or(u64::MAX),
        });
        if cancellation.is_cancelled() || control.is_cancelled(RawBayerStage::Inference) {
            return Err(RawBayerWorkflowError::Cancelled(RawBayerStage::Inference));
        }
        let input = RawBayerTileInput::new(tile.clone(), &tensor);
        let result = model.infer(provider, &input, cancellation);
        let output = match result {
            Ok(value) => value,
            Err(RawBayerModelError::Cancelled) => {
                return Err(RawBayerWorkflowError::Cancelled(RawBayerStage::Inference));
            }
            Err(_error)
                if request.provider() == ProviderPolicy::Auto && provider != Provider::Cpu =>
            {
                provider = Provider::Cpu;
                model
                    .infer(provider, &input, cancellation)
                    .map_err(|retry| match retry {
                        RawBayerModelError::Cancelled => {
                            RawBayerWorkflowError::Cancelled(RawBayerStage::Inference)
                        }
                        value => RawBayerWorkflowError::Model(value),
                    })?
            }
            Err(error) => return Err(RawBayerWorkflowError::Model(error)),
        };
        let expected = usize::try_from(model.descriptor().tile_width)
            .ok()
            .and_then(|w| {
                usize::try_from(model.descriptor().tile_height)
                    .ok()
                    .and_then(|h| w.checked_mul(h))
            })
            .and_then(|n| n.checked_mul(4))
            .ok_or(RawBayerWorkflowError::Plan(
                super::planning::RawBayerPlanError::ArithmeticOverflow,
            ))?;
        if output.len() != expected || output.iter().any(|value| !value.is_finite()) {
            return Err(RawBayerWorkflowError::Model(
                RawBayerModelError::InvalidOutput,
            ));
        }
        let crop = model.descriptor().valid_crop;
        let (ox, oy) = tile.output_origin();
        let (ow, oh) = tile.output_dimensions();
        let (ix, iy) = tile.input_origin();
        let tile_width = model.descriptor().tile_width;
        for y in 0..oh {
            for x in 0..ow {
                let gx = ox + x;
                let gy = oy + y;
                let global = usize::try_from(gy)
                    .ok()
                    .and_then(|row| row.checked_mul(usize::try_from(packed.width()).ok()?))
                    .and_then(|i| i.checked_add(usize::try_from(gx).ok()?))
                    .ok_or(RawBayerWorkflowError::Plan(
                        super::planning::RawBayerPlanError::ArithmeticOverflow,
                    ))?;
                let tx = (ox + x)
                    .saturating_sub(ix)
                    .saturating_add(crop.left)
                    .min(tile_width - 1);
                let ty = (oy + y)
                    .saturating_sub(iy)
                    .saturating_add(crop.top)
                    .min(model.descriptor().tile_height - 1);
                let local = usize::try_from(ty)
                    .ok()
                    .and_then(|row| row.checked_mul(usize::try_from(tile_width).ok()?))
                    .and_then(|i| i.checked_add(usize::try_from(tx).ok()?))
                    .ok_or(RawBayerWorkflowError::Plan(
                        super::planning::RawBayerPlanError::ArithmeticOverflow,
                    ))?;
                let weight = blend_weight(
                    tx,
                    ty,
                    model.descriptor().tile_width,
                    model.descriptor().tile_height,
                    model.descriptor().overlap,
                );
                weights[global] += f64::from(weight);
                for plane in 0..4 {
                    accum[plane * plane_size + global] +=
                        f64::from(output[plane * expected / 4 + local]) * f64::from(weight);
                }
            }
        }
        observer.progress(RawBayerProgress {
            stage: RawBayerStage::Inference,
            completed: u64::try_from(index + 1).unwrap_or(u64::MAX),
            total: u64::try_from(plan.tiles().len()).unwrap_or(u64::MAX),
        });
    }
    if cancellation.is_cancelled() || control.is_cancelled(RawBayerStage::Blend) {
        return Err(RawBayerWorkflowError::Cancelled(RawBayerStage::Blend));
    }
    let mut result = vec![0.0; plane_size * 4];
    for global in 0..plane_size {
        if weights[global] == 0.0 {
            return Err(RawBayerWorkflowError::NonFiniteOutput);
        }
        for plane in 0..4 {
            result[plane * plane_size + global] =
                (accum[plane * plane_size + global] / weights[global]) as f32;
            if !result[plane * plane_size + global].is_finite() {
                return Err(RawBayerWorkflowError::NonFiniteOutput);
            }
        }
    }
    observer.progress(RawBayerProgress {
        stage: RawBayerStage::Blend,
        completed: 1,
        total: 1,
    });
    let source = source_packed(
        request.source(),
        plan.processing_area(),
        packed,
        model.descriptor(),
    )?;
    let strength = f64::from(request.strength().value());
    for (value, source_value) in result.iter_mut().zip(source) {
        *value = (f64::from(*value) * strength + f64::from(source_value) * (1.0 - strength)) as f32;
    }
    Ok((result, provider))
}

fn pack_tile(
    frame: &RawFrame,
    area: rusttable_image::Roi,
    tile: &RawBayerTile,
    descriptor: &RawBayerModelDescriptor,
) -> Result<Vec<f32>, super::types::RawFrameError> {
    let width = usize::try_from(descriptor.tile_width)
        .map_err(|_| super::types::RawFrameError::ArithmeticOverflow)?;
    let height = usize::try_from(descriptor.tile_height)
        .map_err(|_| super::types::RawFrameError::ArithmeticOverflow)?;
    let plane_size = width
        .checked_mul(height)
        .ok_or(super::types::RawFrameError::ArithmeticOverflow)?;
    let mut tensor = vec![0.0; plane_size * 4];
    let phase = frame
        .raw()
        .pattern()
        .phase_after_crop(frame.raw().phase(), area);
    let offsets = plane_offsets(frame.raw().pattern(), phase);
    let area_w = area.width();
    let area_h = area.height();
    for plane in 0..4 {
        for y in 0..descriptor.tile_height {
            for x in 0..descriptor.tile_width {
                let px = reflect(tile.input_origin().0.saturating_add(x), area_w.div_ceil(2));
                let py = reflect(tile.input_origin().1.saturating_add(y), area_h.div_ceil(2));
                let lx = (px * 2 + offsets[plane].0).min(area_w - 1);
                let ly = (py * 2 + offsets[plane].1).min(area_h - 1);
                let full_x = area.x() + lx;
                let full_y = area.y() + ly;
                let index = usize::try_from(full_y)
                    .ok()
                    .and_then(|row| row.checked_mul(frame.raw().row_stride_samples()))
                    .and_then(|i| i.checked_add(usize::try_from(full_x).ok()?))
                    .ok_or(super::types::RawFrameError::ArithmeticOverflow)?;
                let value = normalise(frame, plane, frame.raw().samples()[index], descriptor);
                tensor[plane * plane_size
                    + usize::try_from(y)
                        .ok()
                        .and_then(|row| row.checked_mul(width))
                        .and_then(|i| i.checked_add(usize::try_from(x).ok()?))
                        .ok_or(super::types::RawFrameError::ArithmeticOverflow)?] = value;
            }
        }
    }
    Ok(tensor)
}

fn source_packed(
    frame: &RawFrame,
    area: rusttable_image::Roi,
    dimensions: rusttable_image::ImageDimensions,
    descriptor: &RawBayerModelDescriptor,
) -> Result<Vec<f32>, RawBayerWorkflowError> {
    let tile = RawBayerTile::new(
        0,
        0,
        dimensions.width(),
        dimensions.height(),
        0,
        0,
        dimensions.width(),
        dimensions.height(),
    );
    pack_tile(
        frame,
        area,
        &tile,
        &RawBayerModelDescriptor {
            tile_width: dimensions.width(),
            tile_height: dimensions.height(),
            ..*descriptor
        },
    )
    .map_err(|_| {
        RawBayerWorkflowError::Plan(super::planning::RawBayerPlanError::ArithmeticOverflow)
    })
}

fn inverse_and_quantize(
    frame: &RawFrame,
    plan: &RawBayerPlan,
    descriptor: &RawBayerModelDescriptor,
    packed: &[f32],
    cancellation: &CancellationToken,
    control: &dyn RawBayerControl,
) -> Result<CfaBayerU16, RawBayerWorkflowError> {
    let mut samples = frame.raw().samples().to_vec();
    let area = plan.processing_area();
    let phase = frame
        .raw()
        .pattern()
        .phase_after_crop(frame.raw().phase(), area);
    let packed_width = plan.packed_dimensions().width() as usize;
    for y in 0..area.height() {
        for x in 0..area.width() {
            if cancellation.is_cancelled() || control.is_cancelled(RawBayerStage::Inverse) {
                return Err(RawBayerWorkflowError::Cancelled(RawBayerStage::Inverse));
            }
            let full_x = area.x() + x;
            let full_y = area.y() + y;
            if frame.masked_areas().iter().any(|roi| {
                full_x >= roi.x()
                    && full_x < roi.right()
                    && full_y >= roi.y()
                    && full_y < roi.bottom()
            }) {
                continue;
            }
            let plane = plane_for(frame.raw().pattern(), phase, x, y);
            let cell = usize::try_from(y / 2)
                .ok()
                .and_then(|row| row.checked_mul(packed_width))
                .and_then(|i| i.checked_add(usize::try_from(x / 2).ok()?))
                .ok_or(RawBayerWorkflowError::Plan(
                    super::planning::RawBayerPlanError::ArithmeticOverflow,
                ))?;
            let blended = packed[plane * packed.len() / 4 + cell];
            if !blended.is_finite() {
                return Err(RawBayerWorkflowError::NonFiniteOutput);
            }
            let gain = frame.calibration().normalized_gains()[plane];
            let value = ((f64::from(blended) - f64::from(descriptor.offset))
                / f64::from(descriptor.scale)
                / f64::from(gain))
                * f64::from(
                    frame.calibration().white()[plane] - frame.calibration().black()[plane],
                )
                + f64::from(frame.calibration().black()[plane]);
            let value = value.clamp(
                f64::from(frame.calibration().black()[plane]),
                f64::from(frame.calibration().white()[plane]),
            );
            let rounded = round_ties_even(value) as u16;
            let index = usize::try_from(full_y)
                .ok()
                .and_then(|row| row.checked_mul(frame.raw().row_stride_samples()))
                .and_then(|i| i.checked_add(usize::try_from(full_x).ok()?))
                .ok_or(RawBayerWorkflowError::Plan(
                    super::planning::RawBayerPlanError::ArithmeticOverflow,
                ))?;
            samples[index] = rounded;
        }
    }
    CfaBayerU16::new(frame, samples).map_err(|_| RawBayerWorkflowError::QuantizationRange)
}

fn normalise(
    frame: &RawFrame,
    plane: usize,
    sample: u16,
    descriptor: &RawBayerModelDescriptor,
) -> f32 {
    let base = f32::from(sample.saturating_sub(frame.calibration().black()[plane]))
        / f32::from(frame.calibration().white()[plane] - frame.calibration().black()[plane]);
    let value = base
        * if descriptor.white_balanced_input {
            frame.calibration().normalized_gains()[plane]
        } else {
            1.0
        };
    (value * descriptor.scale + descriptor.offset)
        .clamp(descriptor.domain_min, descriptor.domain_max)
}

fn blend_weight(x: u32, y: u32, width: u32, height: u32, overlap: u32) -> f32 {
    if overlap == 0 {
        return 1.0;
    }
    let edge = x.min(y).min(width - 1 - x).min(height - 1 - y);
    (edge.min(overlap) + 1) as f32 / (overlap + 1) as f32
}

fn reflect(value: u32, limit: u32) -> u32 {
    if limit <= 1 {
        return 0;
    }
    let period = 2 * limit - 2;
    let value = value % period;
    if value < limit { value } else { period - value }
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
