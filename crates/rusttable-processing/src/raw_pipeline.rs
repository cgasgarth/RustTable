//! The source-side RAW segment shared by preview and export.
//!
//! RAW white balance is intentionally resolved here, on the normalized CFA
//! plane, before demosaic. The compiled RGB graph then receives a graph with
//! that node disabled so the persisted operation is applied exactly once.

use crate::graph::CompiledOperationGraph;
use crate::operations::OperationExecutionError;
use crate::operations::temperature::{
    TemperatureConfig, TemperaturePlan, TemperaturePlanError, WhiteBalanceReceipt,
    WhiteBalanceStage,
};
use crate::{
    DemosaicAlgorithm, DemosaicError, DemosaicPlan, DemosaicedImage, FiniteF32, RawPrepareConfig,
    RawPrepareError, RawPreparePlan,
};
use rusttable_image::RawMosaicSource;
use std::fmt;

/// The selected persisted RAW temperature node and its graph opacity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawTemperatureSelection {
    config: TemperatureConfig,
    opacity: FiniteF32,
}

impl RawTemperatureSelection {
    #[must_use]
    pub const fn new(config: TemperatureConfig, opacity: FiniteF32) -> Self {
        Self { config, opacity }
    }

    #[must_use]
    pub const fn config(&self) -> &TemperatureConfig {
        &self.config
    }

    #[must_use]
    pub const fn opacity(&self) -> FiniteF32 {
        self.opacity
    }
}

/// Evidence for the ordered source-side RAW segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawPipelineReceipt {
    raw_prepare_identity: [u8; 32],
    temperature: Option<WhiteBalanceReceipt>,
    demosaic_identity: [u8; 32],
    temperature_applied_once: bool,
}

impl RawPipelineReceipt {
    #[must_use]
    pub const fn raw_prepare_identity(self) -> [u8; 32] {
        self.raw_prepare_identity
    }

    #[must_use]
    pub const fn temperature(self) -> Option<WhiteBalanceReceipt> {
        self.temperature
    }

    #[must_use]
    pub const fn demosaic_identity(self) -> [u8; 32] {
        self.demosaic_identity
    }

    #[must_use]
    pub const fn temperature_applied_once(self) -> bool {
        self.temperature_applied_once
    }
}

/// Checked source-side RAW plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawPipelinePlan {
    prepare: RawPreparePlan,
    temperature: Option<TemperaturePlan>,
    opacity: FiniteF32,
    demosaic: DemosaicPlan,
    receipt: RawPipelineReceipt,
}

impl RawPipelinePlan {
    /// Compiles the RAW segment from the source mosaic and the active graph
    /// temperature node. Post-demosaic temperature nodes are deliberately not
    /// consumed here and remain in the RGB graph.
    ///
    /// # Errors
    ///
    /// Returns a typed preparation, temperature, or demosaic planning error.
    ///
    /// # Panics
    ///
    /// The fixed opacity fallback is finite by construction.
    pub fn new(
        source: &RawMosaicSource,
        temperature: Option<RawTemperatureSelection>,
        algorithm: DemosaicAlgorithm,
    ) -> Result<Self, RawPipelineError> {
        let prepare = RawPreparePlan::new(
            source.mosaic(),
            RawPrepareConfig::new(source.default_crop().or(source.active_area())),
        )?;
        let normalized = prepare.execute(source.mosaic())?;
        let (temperature, opacity, temperature_receipt) = match temperature {
            Some(selection) if selection.config().stage() == WhiteBalanceStage::PreDemosaic => {
                let plan = TemperaturePlan::new(selection.config().clone())?;
                let receipt = plan.receipt();
                (Some(plan), selection.opacity(), Some(receipt))
            }
            _ => (None, FiniteF32::new(1.0).expect("finite opacity"), None),
        };
        let demosaic = DemosaicPlan::new(&normalized, algorithm)?;
        let raw_prepare_identity = prepare.identity();
        let demosaic_identity = demosaic.identity();
        Ok(Self {
            prepare,
            temperature,
            opacity,
            demosaic,
            receipt: RawPipelineReceipt {
                raw_prepare_identity,
                temperature: temperature_receipt,
                demosaic_identity,
                temperature_applied_once: temperature_receipt.is_some(),
            },
        })
    }

    #[must_use]
    pub const fn receipt(&self) -> RawPipelineReceipt {
        self.receipt
    }

    /// Executes rawprepare, exactly one optional pre-demosaic temperature, and
    /// demosaic without clipping scene-linear samples.
    ///
    /// # Errors
    ///
    /// Returns a typed preparation, temperature, or demosaic execution error.
    pub fn execute(
        &self,
        source: &RawMosaicSource,
    ) -> Result<RawPipelineExecution, RawPipelineError> {
        let normalized = self.prepare.execute(source.mosaic())?;
        let normalized = if let Some(temperature) = &self.temperature {
            let balanced = temperature.execute_raw(&normalized)?;
            let opacity = self.opacity.get();
            let samples = normalized
                .samples()
                .iter()
                .copied()
                .zip(balanced.samples().iter().copied())
                .map(|(before, after)| {
                    FiniteF32::new(before.get() + (after.get() - before.get()) * opacity)
                        .map_err(|_| RawPipelineError::NonFiniteSample)
                })
                .collect::<Result<Vec<_>, _>>()?;
            normalized.with_samples(samples)?
        } else {
            normalized
        };
        let image = self.demosaic.execute(&normalized)?;
        Ok(RawPipelineExecution {
            image,
            receipt: self.receipt,
        })
    }
}

/// Result of the ordered RAW source segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawPipelineExecution {
    image: DemosaicedImage,
    receipt: RawPipelineReceipt,
}

impl RawPipelineExecution {
    #[must_use]
    pub const fn image(&self) -> &DemosaicedImage {
        &self.image
    }

    #[must_use]
    pub const fn receipt(&self) -> RawPipelineReceipt {
        self.receipt
    }
}

/// Finds the one active pre-demosaic temperature node in a graph.
///
/// # Errors
///
/// Returns an error when the graph contains multiple active pre-demosaic
/// temperature nodes.
pub fn pre_demosaic_temperature(
    graph: &CompiledOperationGraph,
) -> Result<Option<RawTemperatureSelection>, RawPipelineError> {
    let mut selected = None;
    for node in graph.nodes().filter(|node| node.operation().is_enabled()) {
        let crate::ProcessingOperationKind::Temperature { config } = node.operation().kind() else {
            continue;
        };
        if config.stage() != WhiteBalanceStage::PreDemosaic {
            continue;
        }
        if selected.is_some() {
            return Err(RawPipelineError::MultiplePreDemosaicTemperature);
        }
        selected = Some(RawTemperatureSelection::new(
            config.clone(),
            node.operation().opacity(),
        ));
    }
    Ok(selected)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawPipelineError {
    Prepare(RawPrepareError),
    TemperaturePlan(TemperaturePlanError),
    TemperatureExecution(OperationExecutionError),
    Demosaic(DemosaicError),
    NonFiniteSample,
    MultiplePreDemosaicTemperature,
}

impl fmt::Display for RawPipelineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Prepare(error) => write!(formatter, "RAW preparation failed: {error}"),
            Self::TemperaturePlan(error) => {
                write!(formatter, "RAW temperature plan failed: {error}")
            }
            Self::TemperatureExecution(error) => {
                write!(formatter, "RAW temperature failed: {error}")
            }
            Self::Demosaic(error) => write!(formatter, "RAW demosaic failed: {error}"),
            Self::NonFiniteSample => {
                formatter.write_str("RAW temperature produced a non-finite sample")
            }
            Self::MultiplePreDemosaicTemperature => {
                formatter.write_str("RAW graph has more than one pre-demosaic temperature node")
            }
        }
    }
}

impl std::error::Error for RawPipelineError {}

impl From<RawPrepareError> for RawPipelineError {
    fn from(error: RawPrepareError) -> Self {
        Self::Prepare(error)
    }
}

impl From<TemperaturePlanError> for RawPipelineError {
    fn from(error: TemperaturePlanError) -> Self {
        Self::TemperaturePlan(error)
    }
}

impl From<OperationExecutionError> for RawPipelineError {
    fn from(error: OperationExecutionError) -> Self {
        Self::TemperatureExecution(error)
    }
}

impl From<DemosaicError> for RawPipelineError {
    fn from(error: DemosaicError) -> Self {
        Self::Demosaic(error)
    }
}
