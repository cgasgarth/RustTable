use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

use super::model::{CURRENT_VERSION, Configuration};

const MAX_UNKNOWN_BYTES: usize = 1024 * 1024;
const MAX_UNKNOWN_DEPTH: usize = 16;
const MAX_UNKNOWN_KEYS: usize = 10_000;
const MAX_UNKNOWN_STRING: usize = 64 * 1024;
const MAX_PATH_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoStage {
    Read,
    CreateDirectory,
    CreateTemporary,
    Permissions,
    Write,
    Flush,
    Sync,
    Replace,
    SyncDirectory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    Io(IoStage),
    Parse(String),
    FutureVersion(u16),
    UnsupportedVersion(u16),
    Invalid { field: String, reason: &'static str },
    SecretField(String),
    UnknownBounds,
    Conflict,
    Poisoned,
}

impl ConfigError {
    pub fn invalid(field: impl Into<String>, reason: &'static str) -> Self {
        Self::Invalid {
            field: field.into(),
            reason,
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(stage) => write!(f, "configuration I/O failed during {stage:?}"),
            Self::Parse(value) => write!(f, "configuration parse failed: {value}"),
            Self::FutureVersion(value) => write!(
                f,
                "configuration schema {value} is newer than supported schema {CURRENT_VERSION}"
            ),
            Self::UnsupportedVersion(value) => {
                write!(f, "configuration schema {value} is unsupported")
            }
            Self::Invalid { field, reason } => {
                write!(f, "invalid configuration field {field}: {reason}")
            }
            Self::SecretField(field) => {
                write!(f, "secret-like configuration field rejected: {field}")
            }
            Self::UnknownBounds => f.write_str("unknown configuration fields exceed bounds"),
            Self::Conflict => f.write_str("configuration changed on disk during save"),
            Self::Poisoned => f.write_str("configuration service state is unavailable"),
        }
    }
}

impl std::error::Error for ConfigError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigFindingKind {
    UnknownField,
    UnknownEnvironment,
    InvalidOverride,
    Migration,
    PerformanceWarning,
    Fallback,
    GpuUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigFinding {
    pub kind: ConfigFindingKind,
    pub code: String,
    pub field: String,
}

#[derive(Clone, PartialEq)]
pub struct UnknownFields {
    value: toml::Value,
    bytes: usize,
    keys: usize,
}

impl fmt::Debug for UnknownFields {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnknownFields")
            .field("bytes", &self.bytes)
            .field("keys", &self.keys)
            .finish_non_exhaustive()
    }
}

impl UnknownFields {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            value: toml::Value::Table(toml::map::Map::new()),
            bytes: 0,
            keys: 0,
        }
    }

    pub(crate) fn value(&self) -> &toml::Value {
        &self.value
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Defaults,
    UserFile,
    Environment,
    Startup,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverrideValue {
    Text(String),
    Unsigned(u64),
    Boolean(bool),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvironmentOverrides {
    pub config_file: Option<String>,
    pub catalog_file: Option<String>,
    pub log_level: Option<String>,
    pub gpu_mode: Option<String>,
    pub cpu_threads: Option<u16>,
    pub host_memory_mib: Option<u32>,
    pub unknown: Vec<String>,
}

impl EnvironmentOverrides {
    /// Parses the explicit v1 environment registry. No arbitrary field mapping is accepted.
    ///
    /// # Errors
    ///
    /// Returns a typed error for malformed registered values or an unsafe path value.
    pub fn from_pairs<I, K, V>(pairs: I) -> Result<Self, ConfigError>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let mut output = Self::default();
        for (key, value) in pairs {
            let key = key.into();
            let value = value.into();
            match key.as_str() {
                "RUSTTABLE_CONFIG_FILE" => {
                    validate_path_value("RUSTTABLE_CONFIG_FILE", &value)?;
                    output.config_file = Some(value);
                }
                "RUSTTABLE_CATALOG_FILE" => {
                    validate_path_value("RUSTTABLE_CATALOG_FILE", &value)?;
                    output.catalog_file = Some(value);
                }
                "RUSTTABLE_LOG_LEVEL" => output.log_level = Some(value),
                "RUSTTABLE_GPU_MODE" => output.gpu_mode = Some(value),
                "RUSTTABLE_CPU_THREADS" => {
                    output.cpu_threads = Some(parse_unsigned("RUSTTABLE_CPU_THREADS", &value)?);
                }
                "RUSTTABLE_HOST_MEMORY_MIB" => {
                    output.host_memory_mib =
                        Some(parse_unsigned("RUSTTABLE_HOST_MEMORY_MIB", &value)?);
                }
                key if key.starts_with("RUSTTABLE_") => output.unknown.push(key.to_owned()),
                _ => {}
            }
        }
        output.unknown.sort_unstable();
        output.unknown.dedup();
        Ok(output)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LayeredConfiguration {
    pub persisted: Configuration,
    pub effective: Configuration,
    pub unknown: UnknownFields,
    pub findings: Vec<ConfigFinding>,
}

/// Parses one TOML document into a typed configuration value.
///
/// # Errors
///
/// Returns [`ConfigError`] for malformed TOML, duplicate keys, or type errors.
pub fn parse_document<T: DeserializeOwned>(text: &str) -> Result<T, ConfigError> {
    toml::from_str(text).map_err(|error| ConfigError::Parse(error.to_string()))
}

/// Resolves the four fixed layers in ascending precedence order.
///
/// # Errors
///
/// Returns [`ConfigError`] when a layer cannot be parsed, migrated, or validated.
pub fn resolve_layers(
    user_text: Option<&str>,
    environment: &EnvironmentOverrides,
    startup: &BTreeMap<String, OverrideValue>,
) -> Result<(Configuration, UnknownFields, Vec<ConfigFinding>), ConfigError> {
    let layers = resolve_layered(user_text, environment, startup)?;
    Ok((layers.effective, layers.unknown, layers.findings))
}

pub(crate) fn resolve_layered(
    user_text: Option<&str>,
    environment: &EnvironmentOverrides,
    startup: &BTreeMap<String, OverrideValue>,
) -> Result<LayeredConfiguration, ConfigError> {
    let mut findings = Vec::new();
    let user_document = match user_text {
        Some(text) if !text.trim().is_empty() => {
            Some(migrate_document(parse_document(text)?, &mut findings)?)
        }
        Some(_) => Some(default_document()),
        None => None,
    };
    let unknown = match user_document.as_ref() {
        Some(document) => collect_unknown(document, &mut findings)?,
        None => UnknownFields::empty(),
    };
    let persisted = match user_document {
        Some(document) => parse_value(&document)?,
        None => Configuration::default(),
    };
    let mut effective_document = toml::Value::try_from(persisted.clone())
        .map_err(|error| ConfigError::Parse(error.to_string()))?;
    apply_environment(&mut effective_document, environment);
    apply_startup(&mut effective_document, startup, &mut findings);
    let effective = parse_value(&effective_document)?;
    validate(&persisted)?;
    validate(&effective)?;
    for name in &environment.unknown {
        findings.push(ConfigFinding {
            kind: ConfigFindingKind::UnknownEnvironment,
            code: "configuration.unknown_environment".to_owned(),
            field: name.clone(),
        });
    }
    Ok(LayeredConfiguration {
        persisted,
        effective,
        unknown,
        findings,
    })
}

fn parse_value(value: &toml::Value) -> Result<Configuration, ConfigError> {
    parse_document(&toml::to_string(value).map_err(|error| ConfigError::Parse(error.to_string()))?)
}

fn default_document() -> toml::Value {
    toml::Value::try_from(Configuration::default()).expect("configuration defaults serialize")
}

fn migrate_document(
    mut document: toml::Value,
    findings: &mut Vec<ConfigFinding>,
) -> Result<toml::Value, ConfigError> {
    let table = document
        .as_table_mut()
        .ok_or_else(|| ConfigError::invalid("document", "configuration must be a table"))?;
    let version = match table.get("schema_version") {
        None => 0,
        Some(value) => value
            .as_integer()
            .and_then(|value| u16::try_from(value).ok())
            .ok_or_else(|| ConfigError::invalid("schema_version", "must be an unsigned integer"))?,
    };
    if version > CURRENT_VERSION {
        return Err(ConfigError::FutureVersion(version));
    }
    if version == 0 {
        table.insert(
            "schema_version".to_owned(),
            toml::Value::Integer(i64::from(CURRENT_VERSION)),
        );
        migrate_legacy_flat_keys(table);
        findings.push(ConfigFinding {
            kind: ConfigFindingKind::Migration,
            code: "configuration.migrated_v0".to_owned(),
            field: "schema_version".to_owned(),
        });
    }
    Ok(document)
}

fn migrate_legacy_flat_keys(table: &mut toml::map::Map<String, toml::Value>) {
    let mappings = [
        ("theme", "ui", "theme"),
        ("preview_max_edge", "ui", "preview_max_edge"),
        ("cpu_threads", "processing", "cpu_threads"),
        ("host_memory_mib", "processing", "host_memory_mib"),
        ("gpu_mode", "gpu", "mode"),
    ];
    for (legacy, section, current) in mappings {
        if let Some(value) = table.remove(legacy)
            && let Some(section_table) = table
                .entry(section.to_owned())
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
                .as_table_mut()
        {
            section_table.insert(current.to_owned(), value);
        }
    }
}

fn apply_environment(document: &mut toml::Value, environment: &EnvironmentOverrides) {
    set_string(
        document,
        "catalog",
        "path",
        environment.catalog_file.clone(),
    );
    set_string(
        document,
        "diagnostics",
        "level",
        environment.log_level.clone(),
    );
    set_string(document, "gpu", "mode", environment.gpu_mode.clone());
    set_u64(
        document,
        "processing",
        "cpu_threads",
        environment.cpu_threads.map(u64::from),
    );
    set_u64(
        document,
        "processing",
        "host_memory_mib",
        environment.host_memory_mib.map(u64::from),
    );
}

fn apply_startup(
    document: &mut toml::Value,
    startup: &BTreeMap<String, OverrideValue>,
    findings: &mut Vec<ConfigFinding>,
) {
    for (key, value) in startup {
        let applied = apply_ui_override(document, key, value)
            || apply_catalog_override(document, key, value)
            || apply_processing_override(document, key, value)
            || apply_product_override(document, key, value);
        if !applied {
            findings.push(ConfigFinding {
                kind: ConfigFindingKind::InvalidOverride,
                code: "configuration.invalid_override".to_owned(),
                field: key.clone(),
            });
        }
    }
}

fn apply_ui_override(document: &mut toml::Value, key: &str, value: &OverrideValue) -> bool {
    match (key, value) {
        ("ui.theme", OverrideValue::Text(value)) => {
            set_string(document, "ui", "theme", Some(value.clone()))
        }
        ("ui.reduced_motion", OverrideValue::Boolean(value)) => {
            set_bool(document, "ui", "reduced_motion", Some(*value))
        }
        ("ui.sidebar_visible", OverrideValue::Boolean(value)) => {
            set_bool(document, "ui", "sidebar_visible", Some(*value))
        }
        ("ui.preview_max_edge", OverrideValue::Unsigned(value)) => {
            set_u64(document, "ui", "preview_max_edge", Some(*value))
        }
        _ => false,
    }
}

fn apply_catalog_override(document: &mut toml::Value, key: &str, value: &OverrideValue) -> bool {
    match (key, value) {
        ("catalog.path", OverrideValue::Text(value)) => {
            set_string(document, "catalog", "path", Some(value.clone()))
        }
        ("catalog.create_if_missing", OverrideValue::Boolean(value)) => {
            set_bool(document, "catalog", "create_if_missing", Some(*value))
        }
        ("catalog.checkpoint_interval_seconds", OverrideValue::Unsigned(value)) => set_u64(
            document,
            "catalog",
            "checkpoint_interval_seconds",
            Some(*value),
        ),
        ("import.max_concurrent_items", OverrideValue::Unsigned(value)) => {
            set_u64(document, "import", "max_concurrent_items", Some(*value))
        }
        ("import.mode", OverrideValue::Text(value)) => {
            set_string(document, "import", "mode", Some(value.clone()))
        }
        _ => false,
    }
}

fn apply_processing_override(document: &mut toml::Value, key: &str, value: &OverrideValue) -> bool {
    match (key, value) {
        ("processing.cpu_threads", OverrideValue::Unsigned(value)) => {
            set_u64(document, "processing", "cpu_threads", Some(*value))
        }
        ("processing.host_memory_mib", OverrideValue::Unsigned(value)) => {
            set_u64(document, "processing", "host_memory_mib", Some(*value))
        }
        ("processing.preview_quality", OverrideValue::Text(value)) => set_string(
            document,
            "processing",
            "preview_quality",
            Some(value.clone()),
        ),
        ("gpu.mode", OverrideValue::Text(value)) => {
            set_string(document, "gpu", "mode", Some(value.clone()))
        }
        ("gpu.power_preference", OverrideValue::Text(value)) => {
            set_string(document, "gpu", "power_preference", Some(value.clone()))
        }
        ("gpu.adapter_alias", OverrideValue::Text(value)) => {
            set_string(document, "gpu", "adapter_alias", Some(value.clone()))
        }
        ("gpu.hard_budget_mib", OverrideValue::Unsigned(value)) => {
            set_u64(document, "gpu", "hard_budget_mib", Some(*value))
        }
        ("cache.host_cache_mib", OverrideValue::Unsigned(value)) => {
            set_u64(document, "cache", "host_cache_mib", Some(*value))
        }
        ("cache.thumbnail_memory_entries", OverrideValue::Unsigned(value)) => {
            set_u64(document, "cache", "thumbnail_memory_entries", Some(*value))
        }
        _ => false,
    }
}

fn apply_product_override(document: &mut toml::Value, key: &str, value: &OverrideValue) -> bool {
    match (key, value) {
        ("diagnostics.level", OverrideValue::Text(value)) => {
            set_string(document, "diagnostics", "level", Some(value.clone()))
        }
        ("diagnostics.human_log", OverrideValue::Boolean(value)) => {
            set_bool(document, "diagnostics", "human_log", Some(*value))
        }
        ("diagnostics.json_log", OverrideValue::Boolean(value)) => {
            set_bool(document, "diagnostics", "json_log", Some(*value))
        }
        ("diagnostics.recent_event_count", OverrideValue::Unsigned(value)) => {
            set_u64(document, "diagnostics", "recent_event_count", Some(*value))
        }
        ("export.default_png_size", OverrideValue::Text(value)) => {
            set_string(document, "export", "default_png_size", Some(value.clone()))
        }
        ("export.allow_upscale", OverrideValue::Boolean(value)) => {
            set_bool(document, "export", "allow_upscale", Some(*value))
        }
        ("export.collision", OverrideValue::Text(value)) => {
            set_string(document, "export", "collision", Some(value.clone()))
        }
        ("camera.enabled", OverrideValue::Boolean(value)) => {
            set_bool(document, "camera", "enabled", Some(*value))
        }
        ("scripting.lua_enabled", OverrideValue::Boolean(value)) => {
            set_bool(document, "scripting", "lua_enabled", Some(*value))
        }
        ("scripting.wasm_extensions_enabled", OverrideValue::Boolean(value)) => set_bool(
            document,
            "scripting",
            "wasm_extensions_enabled",
            Some(*value),
        ),
        _ => false,
    }
}

fn set_string(document: &mut toml::Value, section: &str, key: &str, value: Option<String>) -> bool {
    if let Some(value) = value
        && let Some(table) = document
            .get_mut(section)
            .and_then(toml::Value::as_table_mut)
    {
        table.insert(key.to_owned(), toml::Value::String(value));
        return true;
    }
    false
}

fn set_u64(document: &mut toml::Value, section: &str, key: &str, value: Option<u64>) -> bool {
    if let Some(value) = value
        && let Some(table) = document
            .get_mut(section)
            .and_then(toml::Value::as_table_mut)
        && let Ok(value) = i64::try_from(value)
    {
        table.insert(key.to_owned(), toml::Value::Integer(value));
        return true;
    }
    false
}

fn set_bool(document: &mut toml::Value, section: &str, key: &str, value: Option<bool>) -> bool {
    if let Some(value) = value
        && let Some(table) = document
            .get_mut(section)
            .and_then(toml::Value::as_table_mut)
    {
        table.insert(key.to_owned(), toml::Value::Boolean(value));
        return true;
    }
    false
}

fn parse_unsigned<T>(field: &str, value: &str) -> Result<T, ConfigError>
where
    T: TryFrom<u64>,
{
    let value = value
        .parse::<u64>()
        .map_err(|_| ConfigError::invalid(field, "must be an unsigned integer"))?;
    T::try_from(value).map_err(|_| ConfigError::invalid(field, "value is out of range"))
}

fn validate(config: &Configuration) -> Result<(), ConfigError> {
    if config.schema_version.0 != CURRENT_VERSION {
        return Err(ConfigError::UnsupportedVersion(config.schema_version.0));
    }
    if !(256..=8192).contains(&config.ui.preview_max_edge) {
        return Err(ConfigError::invalid(
            "ui.preview_max_edge",
            "must be between 256 and 8192",
        ));
    }
    if !(5..=3600).contains(&config.catalog.checkpoint_interval_seconds) {
        return Err(ConfigError::invalid(
            "catalog.checkpoint_interval_seconds",
            "must be between 5 and 3600",
        ));
    }
    if !(1..=16).contains(&config.import.max_concurrent_items) {
        return Err(ConfigError::invalid(
            "import.max_concurrent_items",
            "must be between 1 and 16",
        ));
    }
    if config.processing.cpu_threads > 256 {
        return Err(ConfigError::invalid(
            "processing.cpu_threads",
            "must be at most 256",
        ));
    }
    if config.processing.host_memory_mib != 0
        && !(512..=8192).contains(&config.processing.host_memory_mib)
    {
        return Err(ConfigError::invalid(
            "processing.host_memory_mib",
            "must be zero or between 512 and 8192",
        ));
    }
    if config.cache.host_cache_mib == 0 || config.cache.thumbnail_memory_entries == 0 {
        return Err(ConfigError::invalid(
            "cache",
            "cache limits must be positive",
        ));
    }
    if config.diagnostics.recent_event_count == 0 || config.diagnostics.recent_event_count > 10_000
    {
        return Err(ConfigError::invalid(
            "diagnostics.recent_event_count",
            "must be between 1 and 10000",
        ));
    }
    if let Some(path) = config.catalog.path.as_deref() {
        validate_path_value("catalog.path", path)?;
    }
    if let Some(alias) = config.gpu.adapter_alias.as_deref()
        && (alias.is_empty() || alias.len() > 128 || alias.chars().any(char::is_control))
    {
        return Err(ConfigError::invalid(
            "gpu.adapter_alias",
            "must be a bounded display alias",
        ));
    }
    Ok(())
}

fn validate_path_value(field: &str, value: &str) -> Result<(), ConfigError> {
    if value.is_empty()
        || value.len() > MAX_PATH_BYTES
        || value.contains('\0')
        || value.chars().any(char::is_control)
    {
        return Err(ConfigError::invalid(field, "must be a bounded valid path"));
    }
    Ok(())
}

fn collect_unknown(
    parsed: &toml::Value,
    findings: &mut Vec<ConfigFinding>,
) -> Result<UnknownFields, ConfigError> {
    let value = collect_unknown_table(parsed, &[], findings)?;
    let value = toml::Value::Table(value);
    let (bytes, keys) = bounds(&value, 0)?;
    Ok(UnknownFields { value, bytes, keys })
}

fn collect_unknown_table(
    parsed: &toml::Value,
    path: &[&str],
    findings: &mut Vec<ConfigFinding>,
) -> Result<toml::map::Map<String, toml::Value>, ConfigError> {
    let Some(table) = parsed.as_table() else {
        return Err(ConfigError::invalid(
            "document",
            "configuration must be a table",
        ));
    };
    let known = known_fields(path);
    let mut unknown = toml::map::Map::new();
    for (key, value) in table {
        if known.contains(&key.as_str()) {
            if value.is_table() {
                let mut child_path = path.to_vec();
                child_path.push(key);
                let child = collect_unknown_table(value, &child_path, findings)?;
                if !child.is_empty() {
                    unknown.insert(key.clone(), toml::Value::Table(child));
                }
            }
            continue;
        }
        if secret_name(key) {
            return Err(ConfigError::SecretField(field_name(path, key)));
        }
        reject_secret_descendants(value, path, key)?;
        unknown.insert(key.clone(), value.clone());
        findings.push(ConfigFinding {
            kind: ConfigFindingKind::UnknownField,
            code: "configuration.unknown_field".to_owned(),
            field: field_name(path, key),
        });
    }
    Ok(unknown)
}

fn known_fields(path: &[&str]) -> &'static [&'static str] {
    match path {
        [] => &[
            "schema_version",
            "ui",
            "catalog",
            "import",
            "processing",
            "gpu",
            "cache",
            "diagnostics",
            "export",
            "camera",
            "scripting",
        ],
        ["ui"] => &[
            "theme",
            "reduced_motion",
            "sidebar_visible",
            "preview_max_edge",
        ],
        ["catalog"] => &["path", "create_if_missing", "checkpoint_interval_seconds"],
        ["import"] => &["max_concurrent_items", "mode"],
        ["processing"] => &["cpu_threads", "host_memory_mib", "preview_quality"],
        ["gpu"] => &[
            "mode",
            "power_preference",
            "adapter_alias",
            "hard_budget_mib",
        ],
        ["cache"] => &["host_cache_mib", "thumbnail_memory_entries"],
        ["diagnostics"] => &["level", "human_log", "json_log", "recent_event_count"],
        ["export"] => &["default_png_size", "allow_upscale", "collision"],
        ["camera"] => &["enabled"],
        ["scripting"] => &["lua_enabled", "wasm_extensions_enabled"],
        _ => &[],
    }
}

fn reject_secret_descendants(
    value: &toml::Value,
    path: &[&str],
    key: &str,
) -> Result<(), ConfigError> {
    if let toml::Value::Table(table) = value {
        for (child, value) in table {
            let mut next = path.to_vec();
            next.push(key);
            if secret_name(child) {
                return Err(ConfigError::SecretField(field_name(&next, child)));
            }
            reject_secret_descendants(value, &next, child)?;
        }
    }
    Ok(())
}

fn field_name(path: &[&str], key: &str) -> String {
    path.iter()
        .chain(std::iter::once(&key))
        .copied()
        .collect::<Vec<_>>()
        .join(".")
}

fn bounds(value: &toml::Value, depth: usize) -> Result<(usize, usize), ConfigError> {
    if depth > MAX_UNKNOWN_DEPTH {
        return Err(ConfigError::UnknownBounds);
    }
    match value {
        toml::Value::String(value) if value.len() > MAX_UNKNOWN_STRING => {
            Err(ConfigError::UnknownBounds)
        }
        toml::Value::Float(value) if !value.is_finite() => Err(ConfigError::UnknownBounds),
        toml::Value::Table(table) => {
            let mut bytes = 0;
            let mut keys = table.len();
            for (key, value) in table {
                bytes += key.len();
                let (child_bytes, child_keys) = bounds(value, depth + 1)?;
                bytes += child_bytes;
                keys += child_keys;
            }
            if bytes > MAX_UNKNOWN_BYTES || keys > MAX_UNKNOWN_KEYS {
                Err(ConfigError::UnknownBounds)
            } else {
                Ok((bytes, keys))
            }
        }
        toml::Value::Array(values) => {
            let mut total = 0;
            let mut keys = 0;
            for value in values {
                let (child_bytes, child_keys) = bounds(value, depth + 1)?;
                total += child_bytes;
                keys += child_keys;
            }
            Ok((total, keys))
        }
        _ => Ok((0, 0)),
    }
}

fn secret_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    [
        "token",
        "password",
        "secret",
        "cookie",
        "private_key",
        "credential",
    ]
    .iter()
    .any(|part| name.contains(part))
}

pub(crate) fn merge_unknown(document: &mut toml::Value, unknown: &UnknownFields) {
    merge_tables(document, unknown.value());
}

fn merge_tables(document: &mut toml::Value, unknown: &toml::Value) {
    let (Some(document), Some(unknown)) = (document.as_table_mut(), unknown.as_table()) else {
        return;
    };
    for (key, value) in unknown {
        match (document.get_mut(key), value) {
            (Some(existing), toml::Value::Table(unknown_table)) if existing.is_table() => {
                merge_tables(existing, &toml::Value::Table(unknown_table.clone()));
            }
            (None, value) => {
                document.insert(key.clone(), value.clone());
            }
            _ => {}
        }
    }
}

pub(crate) fn document_hash(config: &Configuration, unknown: &UnknownFields) -> String {
    let document = canonical_document(config, unknown);
    format!("{:x}", Sha256::digest(document.as_bytes()))
}

pub(crate) fn canonical_document(config: &Configuration, unknown: &UnknownFields) -> String {
    let mut document = toml::Value::try_from(config).expect("configuration serializes");
    merge_unknown(&mut document, unknown);
    toml::to_string_pretty(&document).expect("configuration serializes deterministically")
}

#[allow(dead_code)]
fn _known_field_set() -> BTreeSet<&'static str> {
    known_fields(&[]).iter().copied().collect()
}
