use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

use rusttable_scripting::component::{
    CacheKey, CapabilitySet, CompiledArtifactCache, ErrorCode, ExtensionId, ExtensionManifest,
    ExtensionPackage, HostConfig, InvocationRequest, Permission, ResourceKind, ResourceRegistry,
    ScriptLimits, WasmtimeHost, WorldVersion,
};

const VALID_COMPONENT: &[u8] = br#"(component (core module $m (func (export "run"))) (core instance $i (instantiate $m)) (func (export "run") (canon lift (core func $i "run"))))"#;
const TRAPPING_COMPONENT: &[u8] = br#"(component (core module $m (func (export "run") unreachable)) (core instance $i (instantiate $m)) (func (export "run") (canon lift (core func $i "run"))))"#;
static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

fn temp_root(label: &str) -> PathBuf {
    let number = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "rusttable-component-{label}-{}-{number}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("cache directory");
    path
}

fn package(name: &str, component: &[u8]) -> ExtensionPackage {
    let id = ExtensionId::new(name).expect("valid id");
    ExtensionPackage::new(
        ExtensionManifest {
            id,
            name: name.to_owned(),
            version: "1.0.0".to_owned(),
            world: WorldVersion::CURRENT,
            requested_permissions: CapabilitySet::new(),
            limits: ScriptLimits::default(),
        },
        component.to_vec(),
    )
    .expect("valid package")
}

fn host() -> WasmtimeHost {
    let root = temp_root("host");
    WasmtimeHost::new(HostConfig {
        cache_root: root,
        ..HostConfig::default()
    })
    .expect("host")
}

#[tokio::test(flavor = "current_thread")]
async fn extension_host_invokes_a_valid_component_in_a_fresh_store() {
    let host = host();
    let package = package("valid", VALID_COMPONENT);
    let id = package.manifest.id.clone();
    let cache = host.install(package).expect("install");
    assert!(cache.accepted);
    host.enable(&id).expect("enable");
    let receipt = host
        .invoke(InvocationRequest::new(id))
        .await
        .expect("invoke");
    assert_eq!(
        receipt.status,
        rusttable_scripting::component::ReceiptStatus::Completed
    );
    assert!(receipt.receipt_id.starts_with("sha256:"));
}

#[tokio::test(flavor = "current_thread")]
async fn extension_host_disable_cancels_future_invocation_and_reload_changes_generation() {
    let host = host();
    let package = package("lifecycle", VALID_COMPONENT);
    let id = package.manifest.id.clone();
    host.install(package.clone()).expect("install");
    let enabled = host.enable(&id).expect("enable");
    let reloaded = host.replace(package).expect("reload");
    assert!(reloaded.generation > enabled.generation);
    host.disable(&id).expect("disable");
    let error = host
        .invoke(InvocationRequest::new(id))
        .await
        .expect_err("disabled extension");
    assert_eq!(error.code, ErrorCode::PermissionDenied);
}

#[tokio::test(flavor = "current_thread")]
async fn extension_host_cancellation_is_observed_before_guest_entry() {
    let host = host();
    let package = package("cancelled", VALID_COMPONENT);
    let id = package.manifest.id.clone();
    host.install(package).expect("install");
    host.enable(&id).expect("enable");
    let request = InvocationRequest::new(id);
    request.cancellation.cancel();
    let error = host
        .invoke(request)
        .await
        .expect_err("cancelled invocation");
    assert_eq!(error.code, ErrorCode::Cancelled);
}

#[tokio::test(flavor = "current_thread")]
async fn extension_host_repeated_traps_quarantine_only_the_faulting_extension() {
    let host = host();
    let package = package("trapping", TRAPPING_COMPONENT);
    let id = package.manifest.id.clone();
    host.install(package).expect("install");
    host.enable(&id).expect("enable");
    for _ in 0..3 {
        let _ = host.invoke(InvocationRequest::new(id.clone())).await;
    }
    assert_eq!(
        host.state(&id),
        Some(rusttable_scripting::component::ExtensionState::Quarantined)
    );
}

#[test]
fn extension_host_typed_resource_handles_enforce_parent_and_generation_lifetimes() {
    let mut resources = ResourceRegistry::new(2);
    let parent = resources
        .insert(ResourceKind::Storage, None)
        .expect("parent");
    let child = resources
        .insert(ResourceKind::CatalogSnapshot, Some(parent))
        .expect("child");
    assert!(resources.drop_handle(parent).is_err());
    resources.drop_handle(child).expect("child drop");
    resources.drop_handle(parent).expect("parent drop");
    let stale = resources
        .insert(ResourceKind::Storage, None)
        .expect("new handle");
    resources.reset();
    assert!(resources.get(stale).is_err());
}

#[test]
fn extension_host_capability_resolution_is_fail_closed() {
    let mut grants = CapabilitySet::new();
    grants.grant(Permission::CatalogRead);
    assert!(grants.require(Permission::CatalogRead).is_ok());
    assert_eq!(
        grants
            .require(Permission::Network)
            .expect_err("ambient network")
            .code,
        ErrorCode::PermissionDenied
    );
}

#[test]
fn extension_host_malformed_components_are_rejected_before_registry_activation() {
    let host = host();
    let result = host.install(package("malformed", b"not-a-component"));
    assert_eq!(
        result.expect_err("malformed input").code,
        ErrorCode::MalformedComponent
    );
}

#[test]
fn extension_host_cache_rejects_substituted_artifacts() {
    let root = temp_root("cache");
    let cache = CompiledArtifactCache::new(&root).expect("cache");
    let key = CacheKey {
        wasmtime: "46.0.1".to_owned(),
        target: "test".to_owned(),
        engine_config: "config".to_owned(),
        component: "sha256:component".to_owned(),
        world: WorldVersion::CURRENT,
        feature_policy: "policy".to_owned(),
    };
    let receipt = cache.write(key.clone(), b"artifact").expect("write");
    assert_eq!(
        cache.read(&key, &receipt.artifact).expect("read"),
        Some(b"artifact".to_vec())
    );
    assert_eq!(
        cache
            .read(&key, "sha256:forged")
            .expect_err("substitution")
            .code,
        ErrorCode::CacheRejected
    );
    drop(cache);
    fs::remove_dir_all(root).expect("temporary cache directory should be removed");
}
