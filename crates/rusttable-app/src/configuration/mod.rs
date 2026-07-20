use directories::ProjectDirs;
use rusttable_core::config::{ConfigError, ConfigurationService, LoadReport};

pub(crate) fn load() -> Result<LoadReport, ConfigError> {
    let directories = ProjectDirs::from("com", "cgasgarth", "RustTable").ok_or_else(|| {
        ConfigError::invalid("configuration directory", "platform path unavailable")
    })?;
    ConfigurationService::new(directories.config_dir().join("config.toml"))
        .with_catalog_default(directories.data_local_dir().join("catalog.db"))
        .load_initial()
}
