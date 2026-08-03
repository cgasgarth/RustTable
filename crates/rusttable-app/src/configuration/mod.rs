use std::cell::RefCell;

use directories::ProjectDirs;
use rusttable_core::config::{ConfigError, ConfigurationService, LoadReport};
use rusttable_processing::ColorZonesChannel;
use rusttable_ui::iop::colorzones::ColorZonesGraphHeight;

use crate::gtk_controller::colorzones_edit::ColorZonesGuiPreferences;

thread_local! {
    static CONFIGURATION: RefCell<Option<ConfigurationService>> = const { RefCell::new(None) };
}

pub fn load() -> Result<LoadReport, ConfigError> {
    let directories = ProjectDirs::from("com", "cgasgarth", "RustTable").ok_or_else(|| {
        ConfigError::invalid("configuration directory", "platform path unavailable")
    })?;
    let service = ConfigurationService::new(directories.config_dir().join("config.toml"))
        .with_catalog_default(directories.data_local_dir().join("catalog.db"));
    let report = service.load_initial()?;
    CONFIGURATION.with_borrow_mut(|slot| *slot = Some(service));
    Ok(report)
}

pub fn colorzones_gui_preferences() -> ColorZonesGuiPreferences {
    CONFIGURATION.with_borrow(|slot| {
        slot.as_ref()
            .and_then(ConfigurationService::snapshot)
            .map_or_else(ColorZonesGuiPreferences::default, |snapshot| {
                preferences_from_ui(&snapshot.configuration.ui)
            })
    })
}

pub fn persist_colorzones_gui_preferences(
    preferences: ColorZonesGuiPreferences,
) -> Result<(), ConfigError> {
    CONFIGURATION.with_borrow(|slot| {
        let service = slot.as_ref().ok_or(ConfigError::Poisoned)?;
        persist_preferences(service, preferences)
    })
}

fn persist_preferences(
    service: &ConfigurationService,
    preferences: ColorZonesGuiPreferences,
) -> Result<(), ConfigError> {
    let active = service.snapshot().ok_or(ConfigError::Poisoned)?;
    let mut candidate = active.as_ref().clone();
    apply_preferences(&mut candidate.configuration.ui, preferences);
    apply_preferences(&mut candidate.persisted_configuration.ui, preferences);
    service.save(&candidate).map(|_| ())
}

fn apply_preferences(
    ui: &mut rusttable_core::config::UiConfig,
    preferences: ColorZonesGuiPreferences,
) {
    let index = u8::try_from(preferences.output_channel().index())
        .expect("Color Zones channel index fits u8");
    assert!(ui.set_colorzones_output_channel_index(index));
    ui.set_colorzones_graph_logical_height(preferences.graph_height().logical_pixels());
}

fn preferences_from_ui(ui: &rusttable_core::config::UiConfig) -> ColorZonesGuiPreferences {
    let channel = ColorZonesChannel::from_raw(i32::from(ui.colorzones_output_channel_index()))
        .unwrap_or(ColorZonesChannel::Lightness);
    ColorZonesGuiPreferences::new(
        channel,
        ColorZonesGraphHeight::clamped(ui.colorzones_graph_logical_height()),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use rusttable_core::config::EnvironmentOverrides;

    use super::*;

    #[test]
    fn colorzones_preferences_round_trip_without_image_edit_state() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rusttable-colorzones-gui-preferences-{}-{nonce}.toml",
            std::process::id()
        ));
        let service = ConfigurationService::new(&path);
        let report = service
            .load_from(None, &EnvironmentOverrides::default(), &BTreeMap::new())
            .expect("defaults load");
        let before_revision = report.snapshot.revision.clone();
        let preferences = ColorZonesGuiPreferences::new(
            ColorZonesChannel::Hue,
            ColorZonesGraphHeight::new(247).expect("height"),
        );
        persist_preferences(&service, preferences).expect("preferences save");
        let stored = service.snapshot().expect("stored snapshot");
        assert_ne!(stored.revision, before_revision);
        assert_eq!(preferences_from_ui(&stored.configuration.ui), preferences);

        let reloaded = ConfigurationService::new(&path)
            .load_initial()
            .expect("preferences reload");
        assert_eq!(
            preferences_from_ui(&reloaded.snapshot.configuration.ui),
            preferences
        );
        let _ = fs::remove_file(path);
    }
}
