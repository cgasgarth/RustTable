//! The one typed configuration boundary shared by `RustTable` services.
mod model;
mod parse;
mod service;

pub use model::{
    CacheConfig, CameraConfig, CatalogConfig, CollisionPolicy, ConfigVersion, Configuration,
    ConfigurationVersion, DiagnosticLevel, DiagnosticsConfig, ExportConfig, GpuConfig, GpuMode,
    ImportConfig, ImportMode, PngSize, PowerPreference, PreviewQuality, ProcessingConfig,
    ScriptingConfig, Theme, UiConfig,
};
pub use parse::{
    ConfigError, ConfigFinding, ConfigFindingKind, EnvironmentOverrides, IoStage, Layer,
    OverrideValue, UnknownFields, parse_document, resolve_layers,
};
pub use service::{
    ComputeBackend, ConfigChange, ConfigRevision, ConfigSnapshot, ConfigurationService, LoadReport,
    ResolvedConfiguration, RuntimeCapabilities, SaveResult,
};
