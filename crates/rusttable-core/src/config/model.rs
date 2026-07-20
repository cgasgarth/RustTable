use serde::{Deserialize, Serialize};

pub const CURRENT_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigurationVersion(pub u16);

impl ConfigurationVersion {
    #[must_use]
    pub const fn current() -> Self {
        Self(CURRENT_VERSION)
    }
}

pub type ConfigVersion = ConfigurationVersion;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Configuration {
    #[serde(default = "default_version")]
    pub schema_version: ConfigurationVersion,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub catalog: CatalogConfig,
    #[serde(default)]
    pub import: ImportConfig,
    #[serde(default)]
    pub processing: ProcessingConfig,
    #[serde(default)]
    pub gpu: GpuConfig,
    #[serde(default)]
    pub cache: CacheConfig,
    #[serde(default)]
    pub diagnostics: DiagnosticsConfig,
    #[serde(default)]
    pub export: ExportConfig,
    #[serde(default)]
    pub camera: CameraConfig,
    #[serde(default)]
    pub scripting: ScriptingConfig,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            schema_version: ConfigurationVersion::current(),
            ui: UiConfig::default(),
            catalog: CatalogConfig::default(),
            import: ImportConfig::default(),
            processing: ProcessingConfig::default(),
            gpu: GpuConfig::default(),
            cache: CacheConfig::default(),
            diagnostics: DiagnosticsConfig::default(),
            export: ExportConfig::default(),
            camera: CameraConfig::default(),
            scripting: ScriptingConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default)]
    pub theme: Theme,
    #[serde(default)]
    pub reduced_motion: bool,
    #[serde(default = "default_true")]
    pub sidebar_visible: bool,
    #[serde(default = "default_preview_edge")]
    pub preview_max_edge: u32,
}
impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            reduced_motion: false,
            sidebar_visible: true,
            preview_max_edge: default_preview_edge(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogConfig {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default = "default_true")]
    pub create_if_missing: bool,
    #[serde(default = "default_checkpoint")]
    pub checkpoint_interval_seconds: u32,
}
impl Default for CatalogConfig {
    fn default() -> Self {
        Self {
            path: None,
            create_if_missing: true,
            checkpoint_interval_seconds: default_checkpoint(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportConfig {
    #[serde(default = "default_four")]
    pub max_concurrent_items: u16,
    #[serde(default)]
    pub mode: ImportMode,
}
impl Default for ImportConfig {
    fn default() -> Self {
        Self {
            max_concurrent_items: default_four(),
            mode: ImportMode::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProcessingConfig {
    #[serde(default)]
    pub cpu_threads: u16,
    #[serde(default)]
    pub host_memory_mib: u32,
    #[serde(default)]
    pub preview_quality: PreviewQuality,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GpuConfig {
    #[serde(default)]
    pub mode: GpuMode,
    #[serde(default)]
    pub power_preference: PowerPreference,
    #[serde(default)]
    pub adapter_alias: Option<String>,
    #[serde(default)]
    pub hard_budget_mib: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheConfig {
    #[serde(default = "default_cache")]
    pub host_cache_mib: u32,
    #[serde(default = "default_thumbnail")]
    pub thumbnail_memory_entries: u32,
}
impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            host_cache_mib: default_cache(),
            thumbnail_memory_entries: default_thumbnail(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticsConfig {
    #[serde(default)]
    pub level: DiagnosticLevel,
    #[serde(default = "default_true")]
    pub human_log: bool,
    #[serde(default = "default_true")]
    pub json_log: bool,
    #[serde(default = "default_recent")]
    pub recent_event_count: u32,
}
impl Default for DiagnosticsConfig {
    fn default() -> Self {
        Self {
            level: DiagnosticLevel::default(),
            human_log: true,
            json_log: true,
            recent_event_count: default_recent(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ExportConfig {
    #[serde(default)]
    pub default_png_size: PngSize,
    #[serde(default)]
    pub allow_upscale: bool,
    #[serde(default)]
    pub collision: CollisionPolicy,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}
impl Default for CameraConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptingConfig {
    #[serde(default = "default_true")]
    pub lua_enabled: bool,
    #[serde(default = "default_true")]
    pub wasm_extensions_enabled: bool,
}
impl Default for ScriptingConfig {
    fn default() -> Self {
        Self {
            lua_enabled: true,
            wasm_extensions_enabled: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImportMode {
    #[default]
    ReferenceInPlace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PreviewQuality {
    Draft,
    #[default]
    Balanced,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GpuMode {
    #[default]
    Auto,
    Cpu,
    Gpu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PowerPreference {
    #[default]
    HighPerformance,
    LowPower,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PngSize {
    #[default]
    Fit4096,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CollisionPolicy {
    #[default]
    CreateNew,
}

fn default_true() -> bool {
    true
}
fn default_version() -> ConfigurationVersion {
    ConfigurationVersion(CURRENT_VERSION)
}
fn default_preview_edge() -> u32 {
    1536
}
fn default_checkpoint() -> u32 {
    60
}
fn default_four() -> u16 {
    4
}
fn default_cache() -> u32 {
    512
}
fn default_thumbnail() -> u32 {
    200
}
fn default_recent() -> u32 {
    2000
}
