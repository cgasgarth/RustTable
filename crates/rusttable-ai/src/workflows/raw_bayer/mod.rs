mod engine;
mod planning;
mod ports;
mod types;

pub use engine::RawBayerDenoiseWorkflow;
pub use planning::RawBayerPlanError;
pub use ports::{
    BlockingCfaBayerDngPublisher, FileCfaBayerDngPublisher, ImportGroupingOutcome,
    NoopRawBayerControl, NoopRawBayerObserver, PublishedCfaBayer, RawBayerCatalogError,
    RawBayerCatalogPort, RawBayerControl, RawBayerDenoiseModel, RawBayerDngPublisher,
    RawBayerModelDescriptor, RawBayerModelError, RawBayerObserver, RawBayerProgress,
    RawBayerPublishError, RawBayerStage, RawBayerTileInput, RawBayerWorkflowError,
};
pub use types::{
    BayerCalibration, CalibrationError, CfaBayerU16, CfaBayerU16Error, CollisionPolicy, EditError,
    RAW_BAYER_PLAN_VERSION, RAW_BAYER_WORKFLOW_VERSION, RawBayerDenoiseRequest,
    RawBayerEditSnapshot, RawBayerOperation, RawBayerOutputRequest, RawBayerPlan, RawBayerReceipt,
    RawBayerTile, RawFrame, RawFrameError, Strength, StrengthError,
};

static NOOP_OBSERVER: NoopRawBayerObserver = NoopRawBayerObserver;
static NOOP_CONTROL: NoopRawBayerControl = NoopRawBayerControl;
