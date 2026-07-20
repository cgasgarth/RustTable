use std::collections::BTreeMap;
use std::fs;

use rusttable_core::config::{
    ComputeBackend, ConfigurationService, EnvironmentOverrides, GpuMode, OverrideValue,
    RuntimeCapabilities, Theme, resolve_layers,
};

#[test]
fn defaults_are_complete_and_layer_precedence_is_explicit() {
    let environment = EnvironmentOverrides::from_pairs([
        (String::from("RUSTTABLE_GPU_MODE"), String::from("cpu")),
        (String::from("RUSTTABLE_CPU_THREADS"), String::from("8")),
    ])
    .expect("registered overrides");
    let mut startup = BTreeMap::new();
    startup.insert("gpu.mode".to_owned(), OverrideValue::Text("gpu".to_owned()));
    let (configuration, _, _) = resolve_layers(None, &environment, &startup).expect("layers");
    assert_eq!(configuration.gpu.mode, GpuMode::Gpu);
    assert_eq!(configuration.processing.cpu_threads, 8);
    assert_eq!(configuration.ui.preview_max_edge, 1536);
}

#[test]
fn unknown_fields_are_bounded_and_preserved_without_secrets() {
    let environment = EnvironmentOverrides::default();
    let (configuration, unknown, findings) = resolve_layers(
        Some("schema_version = 1\nfuture_setting = \"kept\"\n"),
        &environment,
        &BTreeMap::new(),
    )
    .expect("unknown field");
    assert_eq!(configuration.schema_version.0, 1);
    assert_eq!(findings.len(), 1);
    assert!(format!("{unknown:?}").contains("bytes"));
    assert!(
        resolve_layers(
            Some("schema_version = 1\napi_token = \"never\"\n"),
            &environment,
            &BTreeMap::new()
        )
        .is_err()
    );
}

#[test]
fn invalid_reload_preserves_active_snapshot_and_save_is_atomic() {
    let directory = std::env::temp_dir().join(format!("rusttable-config-{}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).expect("directory");
    let path = directory.join("config.toml");
    let service = ConfigurationService::new(&path);
    let initial = service.load_initial().expect("defaults");
    service.save(&initial.snapshot).expect("create config");
    let saved = fs::read_to_string(&path).expect("saved config");
    assert!(saved.contains("schema_version = 1"));
    fs::write(&path, "schema_version = 1\n[ui]\npreview_max_edge = 1\n").expect("corrupt config");
    assert!(
        resolve_layers(
            Some("schema_version = 1\n[ui]\npreview_max_edge = 1\n"),
            &EnvironmentOverrides::default(),
            &BTreeMap::new()
        )
        .is_err()
    );
    assert!(service.reload().is_err());
    assert_eq!(
        service.snapshot().expect("active").revision,
        initial.snapshot.revision
    );
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn nested_unknown_values_round_trip_and_runtime_overrides_are_not_persisted() {
    let directory =
        std::env::temp_dir().join(format!("rusttable-config-nested-{}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).expect("directory");
    let path = directory.join("config.toml");
    let service = ConfigurationService::new(&path).with_runtime_capabilities(RuntimeCapabilities {
        physical_memory_mib: Some(16_384),
        qualified_gpu: false,
    });
    let environment = EnvironmentOverrides::default();
    let mut startup = BTreeMap::new();
    startup.insert("gpu.mode".to_owned(), OverrideValue::Text("gpu".to_owned()));
    let report = service
        .load_from(
            Some("schema_version = 1\n[ui]\npreview_max_edge = 2048\n[future]\nanswer = 42\n[future.nested]\nname = \"kept\"\n"),
            &environment,
            &startup,
        )
        .expect("load");
    assert_eq!(report.snapshot.configuration.gpu.mode, GpuMode::Gpu);
    assert_eq!(
        report.snapshot.resolved.compute_backend,
        ComputeBackend::CpuFallback
    );
    service
        .save(&report.snapshot)
        .expect("save persisted layer");
    let saved = fs::read_to_string(&path).expect("saved config");
    assert!(saved.contains("[future.nested]"));
    assert!(saved.contains("name = \"kept\""));
    assert_eq!(saved.matches("mode = \"gpu\"").count(), 0);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn registered_environment_values_are_typed_and_unknown_names_are_reported() {
    assert!(
        EnvironmentOverrides::from_pairs([("RUSTTABLE_CPU_THREADS", "not-a-number",)]).is_err()
    );
    let environment = EnvironmentOverrides::from_pairs([
        ("RUSTTABLE_UNKNOWN", "private-value"),
        ("RUSTTABLE_GPU_MODE", "cpu"),
    ])
    .expect("known values");
    let (_, _, findings) = resolve_layers(None, &environment, &BTreeMap::new()).expect("layers");
    assert_eq!(findings[0].field, "RUSTTABLE_UNKNOWN");
}

#[test]
fn v0_migration_and_future_versions_are_explicit() {
    let environment = EnvironmentOverrides::default();
    let migrated = resolve_layers(
        Some("theme = \"dark\"\npreview_max_edge = 2048\n"),
        &environment,
        &BTreeMap::new(),
    )
    .expect("v0 migration");
    assert_eq!(migrated.0.ui.theme, Theme::Dark);
    assert!(
        migrated
            .2
            .iter()
            .any(|finding| finding.code == "configuration.migrated_v0")
    );
    assert!(
        resolve_layers(
            Some("schema_version = 99\n"),
            &environment,
            &BTreeMap::new(),
        )
        .is_err()
    );
}

#[test]
fn duplicate_keys_and_external_writes_are_rejected_without_replacing_active_state() {
    let directory =
        std::env::temp_dir().join(format!("rusttable-config-conflict-{}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).expect("directory");
    let path = directory.join("config.toml");
    let service = ConfigurationService::new(&path);
    let report = service
        .load_from(
            Some("schema_version = 1\n"),
            &EnvironmentOverrides::default(),
            &BTreeMap::new(),
        )
        .expect("load");
    let subscriber = service.subscribe();
    service.save(&report.snapshot).expect("save");
    assert!(subscriber.recv().is_ok());
    let original = service.snapshot().expect("active").revision.clone();
    fs::write(&path, "schema_version = 1\n[ui]\npreview_max_edge = 2048\n")
        .expect("external write");
    assert!(service.save(&report.snapshot).is_err());
    assert_eq!(service.snapshot().expect("active").revision, original);
    assert!(
        resolve_layers(
            Some("schema_version = 1\n[ui]\npreview_max_edge = 256\npreview_max_edge = 512\n"),
            &EnvironmentOverrides::default(),
            &BTreeMap::new(),
        )
        .is_err()
    );
    let _ = fs::remove_dir_all(directory);
}

#[cfg(unix)]
#[test]
fn configuration_symlinks_are_not_followed() {
    use std::os::unix::fs::symlink;

    let directory =
        std::env::temp_dir().join(format!("rusttable-config-symlink-{}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).expect("directory");
    let target = directory.join("real.toml");
    let path = directory.join("config.toml");
    fs::write(&target, "schema_version = 1\n").expect("target");
    symlink(&target, &path).expect("symlink");
    assert!(ConfigurationService::new(&path).load_initial().is_err());
    let _ = fs::remove_dir_all(directory);
}
