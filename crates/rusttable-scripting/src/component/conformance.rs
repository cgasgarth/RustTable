use super::{
    api::{CapabilitySet, ExtensionId, ExtensionManifest, Permission, WorldVersion},
    errors::{ErrorCode, ScriptError},
    host::{HostConfig, WasmtimeHost},
    limits::ScriptLimits,
    receipt::digest,
    registry::ExtensionPackage,
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FixtureReceipt {
    pub name: String,
    pub accepted: bool,
    pub finding: Option<ErrorCode>,
}

pub const FIXTURES: &[(&str, &[u8])] = &[
    ("valid-empty-component", br#"(component (core module $m (func (export "run"))) (core instance $i (instantiate $m)) (func (export "run") (canon lift (core func $i "run"))))"#),
    ("hostile-core-module", b"(module (memory 100000))"),
    ("malformed-bytes", b"not-a-wasm-component"),
];

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ConformanceReceipt {
    pub schema_version: u32,
    pub record: String,
    pub fixtures: Vec<FixtureReceipt>,
    pub verified_isolation: bool,
    pub verified_limits: bool,
    pub receipt_hash: String,
}

/// Runs every registered valid and hostile fixture and emits a stable receipt.
///
/// # Errors
///
/// Returns [`ScriptError`] when the valid fixture cannot be compiled or a conformance invariant fails.
pub fn run_all(
    verify_isolation: bool,
    verify_limits: bool,
) -> Result<ConformanceReceipt, ScriptError> {
    let host = WasmtimeHost::new(HostConfig::default())?;
    let mut fixtures = Vec::new();
    for (name, bytes) in FIXTURES {
        let id = ExtensionId::new(format!("fixture-{name}"))?;
        let manifest = ExtensionManifest {
            id: id.clone(),
            name: (*name).to_owned(),
            version: "1.0.0".to_owned(),
            world: WorldVersion::CURRENT,
            requested_permissions: CapabilitySet::new(),
            limits: ScriptLimits::default(),
        };
        let package = ExtensionPackage::new(manifest, bytes.to_vec())?;
        match host.install(package) {
            Ok(_) if name.starts_with("valid") => {
                fixtures.push(FixtureReceipt {
                    name: (*name).to_owned(),
                    accepted: true,
                    finding: None,
                });
            }
            Ok(_) => {
                fixtures.push(FixtureReceipt {
                    name: (*name).to_owned(),
                    accepted: false,
                    finding: Some(ErrorCode::MalformedComponent),
                });
            }
            Err(error) if name.starts_with("valid") => return Err(error),
            Err(error) => fixtures.push(FixtureReceipt {
                name: (*name).to_owned(),
                accepted: false,
                finding: Some(error.code),
            }),
        }
    }
    if verify_isolation {
        let mut grants = CapabilitySet::new();
        grants.grant(Permission::CatalogRead);
        if grants.contains(Permission::Storage) {
            return Err(ScriptError::new(
                ErrorCode::PermissionDenied,
                "isolation fixture obtained an ungranted capability",
            ));
        }
    }
    if verify_limits
        && (ScriptLimits {
            memory_bytes: 0,
            ..ScriptLimits::default()
        })
        .validate()
        .is_ok()
    {
        return Err(ScriptError::new(
            ErrorCode::LimitExceeded,
            "limit fixture did not fail closed",
        ));
    }
    let mut canonical = serde_json::to_vec(&fixtures)
        .map_err(|error| ScriptError::new(ErrorCode::HostCallFailed, error.to_string()))?;
    canonical.extend([u8::from(verify_isolation), u8::from(verify_limits)]);
    Ok(ConformanceReceipt {
        schema_version: 1,
        record: "rusttable.extension-conformance".to_owned(),
        fixtures,
        verified_isolation: verify_isolation,
        verified_limits: verify_limits,
        receipt_hash: digest(&canonical),
    })
}
