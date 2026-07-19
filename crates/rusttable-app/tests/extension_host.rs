use rusttable_app::ApplicationExtensions;
use rusttable_scripting::component::{
    CapabilitySet, ExtensionId, ExtensionManifest, ExtensionPackage, HostConfig, ScriptLimits,
    WorldVersion,
};

const VALID_COMPONENT: &[u8] = br#"(component (core module $m (func (export "run"))) (core instance $i (instantiate $m)) (func (export "run") (canon lift (core func $i "run"))))"#;

#[test]
fn extension_host_app_composition_owns_extension_lifecycle() {
    let root = tempfile::tempdir().expect("cache directory").keep();
    let app = ApplicationExtensions::new(HostConfig {
        cache_root: root,
        ..HostConfig::default()
    })
    .expect("host");
    let id = ExtensionId::new("app-extension").expect("id");
    let package = ExtensionPackage::new(
        ExtensionManifest {
            id: id.clone(),
            name: "app-extension".to_owned(),
            version: "1.0.0".to_owned(),
            world: WorldVersion::CURRENT,
            requested_permissions: CapabilitySet::new(),
            limits: ScriptLimits::default(),
        },
        VALID_COMPONENT.to_vec(),
    )
    .expect("package");
    app.install(package).expect("install");
    app.enable(&id).expect("enable");
    assert_eq!(
        app.state(&id),
        Some(rusttable_scripting::component::ExtensionState::Enabled)
    );
    app.disable(&id).expect("disable");
    assert_eq!(
        app.state(&id),
        Some(rusttable_scripting::component::ExtensionState::Disabled)
    );
}
