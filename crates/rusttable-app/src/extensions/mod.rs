use rusttable_scripting::component::{
    ExtensionId, ExtensionPackage, HostConfig, InvocationReceipt, InvocationRequest,
    LifecycleReceipt, ScriptError, WasmtimeHost,
};

pub struct ApplicationExtensions {
    host: WasmtimeHost,
}

impl ApplicationExtensions {
    /// Creates the application-owned extension host.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError`] when the Wasmtime engine or cache cannot be initialized.
    pub fn new(config: HostConfig) -> Result<Self, ScriptError> {
        Ok(Self {
            host: WasmtimeHost::new(config)?,
        })
    }
    /// Installs a validated extension package for application lifecycle control.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError`] when package validation or cache persistence fails.
    pub fn install(
        &self,
        package: ExtensionPackage,
    ) -> Result<rusttable_scripting::component::CacheReceipt, ScriptError> {
        self.host.install(package)
    }
    /// Enables one installed extension.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError`] when the extension is unknown or quarantined.
    pub fn enable(&self, extension: &ExtensionId) -> Result<LifecycleReceipt, ScriptError> {
        self.host.enable(extension)
    }
    /// Disables one installed extension and invalidates its pending generation.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError`] when the extension is unknown.
    pub fn disable(&self, extension: &ExtensionId) -> Result<LifecycleReceipt, ScriptError> {
        self.host.disable(extension)
    }
    /// Reloads one extension transactionally.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError`] when replacement validation or lifecycle transition fails.
    pub fn reload(&self, package: ExtensionPackage) -> Result<LifecycleReceipt, ScriptError> {
        self.host.replace(package)
    }
    /// Invokes an extension through the application-owned host.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError`] when lifecycle, capability, limit, cancellation, or guest execution fails.
    pub async fn invoke(
        &self,
        request: InvocationRequest,
    ) -> Result<InvocationReceipt, ScriptError> {
        self.host.invoke(request).await
    }

    #[must_use]
    pub fn state(
        &self,
        extension: &ExtensionId,
    ) -> Option<rusttable_scripting::component::ExtensionState> {
        self.host.state(extension)
    }
}
