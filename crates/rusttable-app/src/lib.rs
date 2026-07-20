#![forbid(unsafe_code)]
#![doc = "The GTK4 application composition root for `RustTable`."]

mod composition;
mod extensions;
pub mod gtk_controller;
pub mod gtk_export;
pub mod gtk_preview_controller;
pub mod library;
pub mod lifecycle;
pub mod macos;
mod platform;
mod preview;
pub mod workspace;

pub use composition::{CatalogPreviewError, CatalogPreviewRequest, CatalogPreviewService, run};
pub use extensions::ApplicationExtensions;
pub use lifecycle::{
    AppServices, ApplicationService, CancellationReason, CancellationToken, LifecycleError,
    LifecycleEvent, LifecycleReceipt, ServiceContext, ServiceCriticality, ServiceDescriptor,
    ServiceError, ServiceErrorKind, ServiceHealth, ServiceHealthFinding, ServiceHealthSnapshot,
    ServiceHealthStatus, ServiceId, ServiceRegistry, ServiceState, ServiceTaskGroup,
    TaskDrainReceipt, TaskGroupError, TaskGroupId, TaskReceipt,
};
pub use preview::{PreviewError, PreviewService};
pub use workspace::{
    BasicEditCommand, BasicEditCommandError, BasicEditCommitError, BasicEditDraft, BasicEditValues,
    commit_basic_edit, commit_basic_edit_at_path,
};

#[cfg(test)]
mod lua_host {
    use std::collections::BTreeSet;

    use rusttable_scripting::{ApiVersion, LuaHost, ScriptId, ScriptLimits, ScriptManifest};

    #[test]
    fn lua_host_is_available_to_the_application_boundary() {
        let source = "return rusttable.application_version()";
        let manifest = ScriptManifest {
            id: ScriptId::new("app-test").expect("script id"),
            name: "Application host test".to_owned(),
            api_min: ApiVersion { major: 1, minor: 0 },
            api_max: ApiVersion { major: 1, minor: 0 },
            permissions: BTreeSet::new(),
            storage_version: 1,
            enabled: true,
            source_sha256: rusttable_scripting::source_hash(source.as_bytes()),
        };
        let mut host = LuaHost::new(
            rusttable_scripting::HostConfig {
                limits: ScriptLimits::default(),
                max_faults: 3,
            },
            rusttable_scripting::HostPorts::default(),
        );
        host.install(&manifest, source).expect("install");
    }
}
