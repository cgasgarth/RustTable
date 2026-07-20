#![forbid(unsafe_code)]
#![doc = "The `RustTable` Iced application composition root."]

mod application;
mod composition;
mod extensions;
mod library;
mod lifecycle;
mod platform;
mod preview;
mod ui_shell;

pub use composition::run;
pub use extensions::ApplicationExtensions;
pub use preview::{PreviewError, PreviewService};

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
