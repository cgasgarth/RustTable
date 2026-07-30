#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
#![doc = "Typed ONNX Runtime boundary for `RustTable` AI services."]

use std::fmt;

/// Providers that are part of the bounded `RustTable` AI contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Provider {
    Cpu,
    CoreMl,
    DirectMl,
    Cuda,
}

/// A safe graph tensor description returned by a native runtime inspector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TensorDescriptor {
    name: String,
    dtype: TensorDataType,
    dimensions: Vec<Dimension>,
}

impl TensorDescriptor {
    #[must_use]
    pub fn new(name: impl Into<String>, dtype: TensorDataType, dimensions: Vec<Dimension>) -> Self {
        Self {
            name: name.into(),
            dtype,
            dimensions,
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn dtype(&self) -> TensorDataType {
        self.dtype
    }

    #[must_use]
    pub fn dimensions(&self) -> &[Dimension] {
        &self.dimensions
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TensorDataType {
    F32,
    F16,
    U8,
    I8,
    I16,
    I32,
    I64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dimension {
    Static(u32),
    Dynamic(String),
}

/// Metadata needed to validate an ONNX graph without exposing native objects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphMetadata {
    ir_version: u32,
    opset: u32,
    operators: Vec<String>,
    inputs: Vec<TensorDescriptor>,
    outputs: Vec<TensorDescriptor>,
    has_external_data: bool,
}

impl GraphMetadata {
    #[must_use]
    pub fn new(
        ir_version: u32,
        opset: u32,
        operators: Vec<String>,
        inputs: Vec<TensorDescriptor>,
        outputs: Vec<TensorDescriptor>,
        has_external_data: bool,
    ) -> Self {
        Self {
            ir_version,
            opset,
            operators,
            inputs,
            outputs,
            has_external_data,
        }
    }

    #[must_use]
    pub const fn ir_version(&self) -> u32 {
        self.ir_version
    }

    #[must_use]
    pub const fn opset(&self) -> u32 {
        self.opset
    }

    #[must_use]
    pub fn operators(&self) -> &[String] {
        &self.operators
    }

    #[must_use]
    pub fn inputs(&self) -> &[TensorDescriptor] {
        &self.inputs
    }

    #[must_use]
    pub fn outputs(&self) -> &[TensorDescriptor] {
        &self.outputs
    }

    #[must_use]
    pub const fn has_external_data(&self) -> bool {
        self.has_external_data
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeIdentity {
    runtime_version: String,
    adapter_version: String,
    target: String,
}

impl RuntimeIdentity {
    #[must_use]
    pub fn new(
        runtime_version: impl Into<String>,
        adapter_version: impl Into<String>,
        target: impl Into<String>,
    ) -> Self {
        Self {
            runtime_version: runtime_version.into(),
            adapter_version: adapter_version.into(),
            target: target.into(),
        }
    }

    #[must_use]
    pub fn runtime_version(&self) -> &str {
        &self.runtime_version
    }

    #[must_use]
    pub fn adapter_version(&self) -> &str {
        &self.adapter_version
    }

    #[must_use]
    pub fn target(&self) -> &str {
        &self.target
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphOptimization {
    Basic,
    Extended,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfiguration {
    optimization: GraphOptimization,
    intra_threads: u32,
    inter_threads: u32,
    sequential: bool,
}

impl RuntimeConfiguration {
    pub const CANONICAL_CPU: Self = Self {
        optimization: GraphOptimization::Basic,
        intra_threads: 1,
        inter_threads: 1,
        sequential: true,
    };

    #[must_use]
    pub const fn optimization(&self) -> GraphOptimization {
        self.optimization
    }

    #[must_use]
    pub const fn intra_threads(&self) -> u32 {
        self.intra_threads
    }

    #[must_use]
    pub const fn inter_threads(&self) -> u32 {
        self.inter_threads
    }

    #[must_use]
    pub const fn sequential(&self) -> bool {
        self.sequential
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterError {
    Unavailable,
    InvalidModel,
    UnsupportedProvider,
    ProviderInitialization,
    Execution,
    Cancelled,
    OutputContract,
    ResourceLimit,
}

impl fmt::Display for AdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => "native ONNX Runtime is unavailable",
            Self::InvalidModel => "native ONNX Runtime rejected the model",
            Self::UnsupportedProvider => "provider is outside the adapter contract",
            Self::ProviderInitialization => "provider initialization failed",
            Self::Execution => "native inference failed",
            Self::Cancelled => "native inference was cancelled",
            Self::OutputContract => "native output violated the tensor contract",
            Self::ResourceLimit => "native resource limit was exceeded",
        })
    }
}

impl std::error::Error for AdapterError {}

/// Product-owned cancellation callback used by native sessions.
pub trait Cancellation: Send + Sync {
    fn is_cancelled(&self) -> bool;
}

#[derive(Debug, Clone, PartialEq)]
pub struct TensorBuffer {
    shape: Vec<u32>,
    values: Vec<f32>,
}

impl TensorBuffer {
    pub fn new(shape: Vec<u32>, values: Vec<f32>) -> Result<Self, AdapterError> {
        let expected = shape
            .iter()
            .try_fold(1_usize, |total, dimension| {
                total.checked_mul(usize::try_from(*dimension).ok()?)
            })
            .ok_or(AdapterError::OutputContract)?;
        if expected != values.len() || values.iter().any(|value| !value.is_finite()) {
            return Err(AdapterError::OutputContract);
        }
        Ok(Self { shape, values })
    }

    #[must_use]
    pub fn shape(&self) -> &[u32] {
        &self.shape
    }

    #[must_use]
    pub fn values(&self) -> &[f32] {
        &self.values
    }
}

/// An owned native session. The implementation may contain runtime handles, but no handle is
/// visible to product crates.
pub trait Session: Send {
    fn run(
        &mut self,
        input: &TensorBuffer,
        cancel: &dyn Cancellation,
    ) -> Result<TensorBuffer, AdapterError>;
    fn terminate(&mut self);
    fn memory_bytes(&self) -> u64;
}

/// The only seam through which product code may ask for ONNX Runtime work.
pub trait RuntimeAdapter: Send + Sync {
    fn identity(&self) -> RuntimeIdentity;
    fn inspect_model(&self, model_bytes: &[u8]) -> Result<GraphMetadata, AdapterError>;
    fn supports(&self, provider: Provider) -> bool;
    fn open_session(
        &self,
        provider: Provider,
        model_bytes: &[u8],
        configuration: &RuntimeConfiguration,
    ) -> Result<Box<dyn Session>, AdapterError>;
}

/// Safe placeholder used on builds where the matching runtime distribution is not installed.
/// It deliberately cannot inspect, execute, or silently emulate ONNX graphs.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnavailableRuntime;

impl RuntimeAdapter for UnavailableRuntime {
    fn identity(&self) -> RuntimeIdentity {
        RuntimeIdentity::new(
            "unavailable",
            "rusttable-ai-native-contract-1",
            std::env::consts::ARCH,
        )
    }

    fn inspect_model(&self, _: &[u8]) -> Result<GraphMetadata, AdapterError> {
        Err(AdapterError::Unavailable)
    }

    fn supports(&self, _: Provider) -> bool {
        false
    }

    fn open_session(
        &self,
        _: Provider,
        _: &[u8],
        _: &RuntimeConfiguration,
    ) -> Result<Box<dyn Session>, AdapterError> {
        Err(AdapterError::Unavailable)
    }
}
