use std::fmt;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use rusttable_image::Roi;

use super::encoding::{CommandEncoder, EncodedBatch, EncodedDispatch};
use sha2::{Digest, Sha256};

use crate::shader::{BindingReflection, BindingResourceKind, ShaderEntry, ShaderReflection};
use crate::transfer::TransferPlan;
use crate::{
    AdapterIdentity, DeviceGeneration, ExecutionTier, FaultState, GpuCapabilitySnapshot,
    GpuFeaturePlan, LimitEnvelope, PipelineCacheIdentity, ResourceClass, ResourceId, ResourceKind,
};

const POINT_WORKGROUP_SIZE: [u32; 3] = [256, 1, 1];
const STORAGE_USAGE: u64 = 1;
const UNIFORM_USAGE: u64 = 2;
const MAX_LABEL_LENGTH: usize = 96;

/// A cancellation signal checked before validation and before command encoding.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScalarValue {
    F32(f32),
    U32(u32),
    I32(i32),
    Bool(bool),
}

impl ScalarValue {
    fn scalar_type(self) -> &'static str {
        match self {
            Self::F32(_) => "f32",
            Self::U32(_) => "u32",
            Self::I32(_) => "i32",
            Self::Bool(_) => "bool",
        }
    }

    fn write(self, output: &mut [u8]) {
        let bytes = match self {
            Self::F32(value) => value.to_bits().to_le_bytes(),
            Self::U32(value) => value.to_le_bytes(),
            Self::I32(value) => value.to_le_bytes(),
            Self::Bool(value) => u32::from(value).to_le_bytes(),
        };
        output.copy_from_slice(&bytes);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParameterValue {
    pub name: String,
    pub value: ScalarValue,
}

impl ParameterValue {
    #[must_use]
    pub fn new(name: impl Into<String>, value: ScalarValue) -> Self {
        Self {
            name: name.into(),
            value,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TypedParameters {
    values: Vec<ParameterValue>,
}

impl TypedParameters {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with(mut self, name: impl Into<String>, value: ScalarValue) -> Self {
        self.values.push(ParameterValue::new(name, value));
        self
    }

    pub fn push(&mut self, name: impl Into<String>, value: ScalarValue) {
        self.values.push(ParameterValue::new(name, value));
    }

    #[must_use]
    pub fn values(&self) -> &[ParameterValue] {
        &self.values
    }

    fn find(&self, name: &str) -> Option<ScalarValue> {
        self.values
            .iter()
            .find(|value| value.name == name)
            .map(|value| value.value)
    }

    fn pack(&self, reflection: &ShaderReflection) -> Result<ParameterBlock, DispatchError> {
        let mut names = std::collections::BTreeSet::new();
        for value in &self.values {
            if !names.insert(value.name.as_str()) {
                return Err(DispatchError::DuplicateParameter(value.name.clone()));
            }
        }
        let uniform_size = reflection
            .bindings
            .iter()
            .filter(|binding| binding.resource == BindingResourceKind::UniformBuffer)
            .map(|binding| binding.minimum_binding_size)
            .max()
            .unwrap_or(0);
        if uniform_size == 0 && !self.values.is_empty() {
            return Err(DispatchError::ParametersWithoutUniform);
        }
        let mut bytes =
            vec![0; usize::try_from(uniform_size).map_err(|_| DispatchError::ArithmeticOverflow)?];
        let mut covered = vec![false; bytes.len()];
        for parameter in &reflection.parameters {
            let end = parameter
                .offset
                .checked_add(parameter.size)
                .ok_or(DispatchError::ArithmeticOverflow)?;
            let start =
                usize::try_from(parameter.offset).map_err(|_| DispatchError::ArithmeticOverflow)?;
            let end = usize::try_from(end).map_err(|_| DispatchError::ArithmeticOverflow)?;
            if end > bytes.len() || parameter.size != 4 {
                return Err(DispatchError::InvalidParameterLayout(
                    parameter.name.clone(),
                ));
            }
            if covered[start..end].iter().any(|used| *used) {
                return Err(DispatchError::OverlappingParameters(parameter.name.clone()));
            }
            covered[start..end].fill(true);
            if parameter.name.starts_with('_') {
                if self.find(&parameter.name).is_some() {
                    return Err(DispatchError::PaddingParameter(parameter.name.clone()));
                }
                continue;
            }
            let value = self
                .find(&parameter.name)
                .ok_or_else(|| DispatchError::MissingParameter(parameter.name.clone()))?;
            if value.scalar_type() != parameter.scalar_type {
                return Err(DispatchError::ParameterType {
                    name: parameter.name.clone(),
                    expected: parameter.scalar_type.clone(),
                    actual: value.scalar_type().to_owned(),
                });
            }
            value.write(&mut bytes[start..end]);
        }
        if let Some(value) = self.values.iter().find(|value| {
            !reflection
                .parameters
                .iter()
                .any(|parameter| parameter.name == value.name)
        }) {
            return Err(DispatchError::UnknownParameter(value.name.clone()));
        }
        let names = format_parameter_names(self);
        let identity = digest(&[b"params", &bytes, &names]);
        Ok(ParameterBlock { bytes, identity })
    }
}

fn format_parameter_names(parameters: &TypedParameters) -> Vec<u8> {
    parameters
        .values
        .iter()
        .fold(Vec::new(), |mut output, value| {
            output.extend_from_slice(value.name.as_bytes());
            output.push(0);
            output.extend_from_slice(value.value.scalar_type().as_bytes());
            output.push(0);
            output
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterBlock {
    bytes: Vec<u8>,
    identity: [u8; 32],
}

impl ParameterBlock {
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Tile {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Tile {
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Result<Self, DispatchError> {
        if width == 0
            || height == 0
            || x.checked_add(width).is_none()
            || y.checked_add(height).is_none()
        {
            return Err(DispatchError::InvalidTile);
        }
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DispatchRegion {
    pub image_width: u32,
    pub image_height: u32,
    pub roi: Roi,
    pub tile: Tile,
}

impl DispatchRegion {
    pub fn new(
        image_width: u32,
        image_height: u32,
        roi: Roi,
        tile: Tile,
    ) -> Result<Self, DispatchError> {
        if image_width == 0 || image_height == 0 || roi.is_empty() {
            return Err(DispatchError::InvalidRegion);
        }
        if roi.right() > image_width || roi.bottom() > image_height {
            return Err(DispatchError::RegionOutOfBounds);
        }
        if roi.x() < tile.x
            || roi.y() < tile.y
            || roi.right() > tile.x + tile.width
            || roi.bottom() > tile.y + tile.height
        {
            return Err(DispatchError::RegionOutsideTile);
        }
        Ok(Self {
            image_width,
            image_height,
            roi,
            tile,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GridPlan {
    pub logical_extent: [u64; 3],
    pub workgroup_size: [u32; 3],
    pub workgroups: [u32; 3],
    pub coverage: [u64; 3],
}

impl GridPlan {
    pub fn checked(
        logical_extent: [u64; 3],
        workgroup_size: [u32; 3],
        workgroups: [u32; 3],
        limits: LimitEnvelope,
    ) -> Result<Self, DispatchError> {
        if logical_extent.contains(&0) || workgroup_size.contains(&0) || workgroups.contains(&0) {
            return Err(DispatchError::InvalidGrid);
        }
        let mut coverage = [0; 3];
        for axis in 0..3 {
            let expected = logical_extent[axis].div_ceil(u64::from(workgroup_size[axis]));
            if expected != u64::from(workgroups[axis]) {
                return Err(DispatchError::GridUndercoverage);
            }
            if workgroups[axis] > limits.max_workgroups_per_dimension {
                return Err(DispatchError::WorkgroupLimit);
            }
            coverage[axis] = u64::from(workgroups[axis])
                .checked_mul(u64::from(workgroup_size[axis]))
                .ok_or(DispatchError::ArithmeticOverflow)?;
        }
        Ok(Self {
            logical_extent,
            workgroup_size,
            workgroups,
            coverage,
        })
    }

    pub fn for_region(
        region: DispatchRegion,
        workgroup_size: [u32; 3],
        limits: LimitEnvelope,
    ) -> Result<Self, DispatchError> {
        let pixels = u64::from(region.roi.width())
            .checked_mul(u64::from(region.roi.height()))
            .ok_or(DispatchError::ArithmeticOverflow)?;
        let workgroups = [
            u32::try_from(pixels.div_ceil(u64::from(workgroup_size[0])))
                .map_err(|_| DispatchError::GridOverflow)?,
            1,
            1,
        ];
        Self::checked([pixels, 1, 1], workgroup_size, workgroups, limits)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingResource {
    pub id: ResourceId,
    pub group: u32,
    pub binding: u32,
    pub class: ResourceClass,
    pub offset: u64,
    pub size: u64,
    pub usage: u64,
    pub resource: BindingResourceKind,
    pub access: String,
    pub address_space: String,
    pub type_description: String,
    pub format: Option<String>,
    pub dimension: Option<String>,
}

impl BindingResource {
    #[must_use]
    pub fn from_reflection(
        reflection: &BindingReflection,
        id: ResourceId,
        class: ResourceClass,
        offset: u64,
        size: u64,
    ) -> Self {
        let usage = match reflection.resource {
            BindingResourceKind::StorageBuffer => STORAGE_USAGE,
            BindingResourceKind::UniformBuffer => UNIFORM_USAGE,
            _ => 0,
        };
        Self {
            id,
            group: reflection.group,
            binding: reflection.binding,
            class,
            offset,
            size,
            usage,
            resource: reflection.resource.clone(),
            access: reflection.access.clone(),
            address_space: reflection.address_space.clone(),
            type_description: reflection.type_description.clone(),
            format: reflection.format.clone(),
            dimension: reflection.dimension.clone(),
        }
    }

    fn validate(
        &self,
        reflection: &BindingReflection,
        generation: DeviceGeneration,
    ) -> Result<(), DispatchError> {
        if self.resource != reflection.resource
            || self.group != reflection.group
            || self.binding != reflection.binding
            || self.access != reflection.access
            || self.address_space != reflection.address_space
            || self.type_description != reflection.type_description
            || self.format != reflection.format
            || self.dimension != reflection.dimension
        {
            return Err(DispatchError::BindingTypeMismatch(reflection.binding));
        }
        if !matches!(
            reflection.resource,
            BindingResourceKind::StorageBuffer | BindingResourceKind::UniformBuffer
        ) || self.id.kind != ResourceKind::Buffer
            || self.class.kind != ResourceKind::Buffer
        {
            return Err(DispatchError::UnsupportedBinding(reflection.binding));
        }
        if self.id.generation != generation || self.class.generation != generation {
            return Err(DispatchError::GenerationMismatch);
        }
        let required_usage = if reflection.resource == BindingResourceKind::StorageBuffer {
            STORAGE_USAGE
        } else {
            UNIFORM_USAGE
        };
        if self.usage & required_usage != required_usage
            || self.class.usage & required_usage != required_usage
        {
            return Err(DispatchError::UsageMismatch(reflection.binding));
        }
        let end = self
            .offset
            .checked_add(self.size)
            .ok_or(DispatchError::ArithmeticOverflow)?;
        if !self
            .offset
            .is_multiple_of(u64::from(reflection.dynamic_offset_alignment.max(1)))
            || !self.offset.is_multiple_of(self.class.alignment.max(1))
            || self.size < u64::from(reflection.minimum_binding_size)
            || end > self.class.size_bytes
        {
            return Err(DispatchError::BindingBounds(reflection.binding));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParityContract {
    pub cpu_reference: String,
    pub tolerance_class: String,
    pub absolute: f32,
    pub relative: f32,
}

impl ParityContract {
    fn validate(&self, reflection: &ShaderReflection) -> Result<(), DispatchError> {
        if self.cpu_reference.is_empty()
            || self.cpu_reference != reflection.numerical.canonical_cpu_reference
            || self.tolerance_class != reflection.numerical.schema_3_tolerance_class
            || !self.absolute.is_finite()
            || !self.relative.is_finite()
            || self.absolute < 0.0
            || self.relative < 0.0
        {
            return Err(DispatchError::ParityMismatch);
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct PrepareRequest<'a> {
    pub operation_id: &'a str,
    pub instance_id: &'a str,
    pub implementation_version: u16,
    pub entry: &'a ShaderEntry,
    pub capabilities: &'a GpuCapabilitySnapshot,
    pub generation: DeviceGeneration,
    pub feature_plan: GpuFeaturePlan,
    pub parameters: TypedParameters,
    pub bindings: Vec<BindingResource>,
    pub region: DispatchRegion,
    pub grid: Option<GridPlan>,
    pub transfers: Vec<TransferPlan>,
    pub parity: ParityContract,
    pub cancellation: CancellationToken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelIdentity {
    pub operation_id: String,
    pub instance_id: String,
    pub implementation_version: u16,
    pub shader: String,
    pub digest: [u8; 32],
}

impl KernelIdentity {
    #[must_use]
    pub fn stable_label(&self) -> String {
        format!("rusttable.dispatch.{}", hex(self.digest))
    }
}

#[derive(Debug, Clone)]
pub struct PreparedGpuKernel {
    pub(super) identity: KernelIdentity,
    entry_point: String,
    source: String,
    parameters: ParameterBlock,
    bindings: Vec<BindingResource>,
    region: DispatchRegion,
    grid: GridPlan,
    transfers: Vec<TransferPlan>,
    parity: ParityContract,
    pub(super) cancellation: CancellationToken,
    pub(super) generation: DeviceGeneration,
    cache_identity: PipelineCacheIdentity,
}

impl PreparedGpuKernel {
    pub fn prepare(request: PrepareRequest<'_>) -> Result<Self, DispatchFailure> {
        let provisional = digest(&[
            b"dispatch",
            request.operation_id.as_bytes(),
            request.instance_id.as_bytes(),
        ]);
        if request.cancellation.is_cancelled() {
            return Err(DispatchFailure::cancelled(provisional));
        }
        validate_request(&request).map_err(|error| DispatchFailure::failed(provisional, error))?;
        let parameters = request
            .parameters
            .pack(&request.entry.reflection)
            .map_err(|error| DispatchFailure::failed(provisional, error))?;
        let bindings = validate_bindings(
            &request.entry.reflection,
            &request.bindings,
            request.generation,
            request.capabilities.limits,
        )
        .map_err(|error| DispatchFailure::failed(provisional, error))?;
        let grid = match request.grid {
            Some(grid) => grid,
            None => GridPlan::for_region(
                request.region,
                request.entry.reflection.workgroup_size,
                request.capabilities.limits,
            )
            .map_err(|error| DispatchFailure::failed(provisional, error))?,
        };
        if grid.workgroup_size != request.entry.reflection.workgroup_size {
            return Err(DispatchFailure::failed(
                provisional,
                DispatchError::GridReflectionMismatch,
            ));
        }
        let kernel_digest = identity_digest(&request, &parameters, &bindings, grid);
        let adapter = request
            .capabilities
            .adapter
            .as_ref()
            .map_or([0; 32], AdapterIdentity::device_key);
        let cache_identity = PipelineCacheIdentity {
            adapter,
            shader: digest(&[request.entry.cache_key().as_bytes()]),
            layout: digest(&[binding_bytes(&bindings).as_slice()]),
            rusttable_version: 1,
        };
        Ok(Self {
            identity: KernelIdentity {
                operation_id: request.operation_id.to_owned(),
                instance_id: request.instance_id.to_owned(),
                implementation_version: request.implementation_version,
                shader: request.entry.id().stable_name(),
                digest: kernel_digest,
            },
            entry_point: request.entry.reflection.entry_point.clone(),
            source: request.entry.expanded_source.clone(),
            parameters,
            bindings,
            region: request.region,
            grid,
            transfers: request.transfers,
            parity: request.parity,
            cancellation: request.cancellation,
            generation: request.generation,
            cache_identity,
        })
    }

    #[must_use]
    pub const fn identity_digest(&self) -> [u8; 32] {
        self.identity.digest
    }

    #[must_use]
    pub fn identity(&self) -> &KernelIdentity {
        &self.identity
    }

    #[must_use]
    pub fn parameter_block(&self) -> &ParameterBlock {
        &self.parameters
    }

    #[must_use]
    pub fn cache_identity(&self) -> &PipelineCacheIdentity {
        &self.cache_identity
    }

    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn encode(&self, encoder: &mut CommandEncoder) -> Result<EncodedBatch, DispatchFailure> {
        if self.cancellation.is_cancelled() {
            return Err(DispatchFailure::cancelled(self.identity.digest));
        }
        let command = self.command();
        encoder
            .append(std::slice::from_ref(&command))
            .map_err(|error| DispatchFailure::failed(self.identity.digest, error))?;
        Ok(EncodedBatch::single(command, self.identity.digest))
    }

    pub(super) fn command(&self) -> EncodedDispatch {
        EncodedDispatch {
            identity: self.identity.digest,
            label: self.identity.stable_label(),
            entry_point: self.entry_point.clone(),
            parameters: self.parameters.clone(),
            bindings: self.bindings.clone(),
            region: self.region,
            grid: self.grid,
            transfers: self.transfers.clone(),
            parity: self.parity.clone(),
        }
    }
}

fn validate_request(request: &PrepareRequest<'_>) -> Result<(), DispatchError> {
    if request.operation_id.is_empty()
        || request.instance_id.is_empty()
        || request.operation_id.len() > MAX_LABEL_LENGTH
        || request.instance_id.len() > MAX_LABEL_LENGTH
    {
        return Err(DispatchError::InvalidIdentity);
    }
    if request.capabilities.is_cpu_only() {
        return Err(DispatchError::CpuRequired);
    }
    if !matches!(
        request.capabilities.state,
        FaultState::Healthy | FaultState::Degraded
    ) {
        return Err(DispatchError::DeviceUnavailable);
    }
    if request.capabilities.generation != request.generation.value() {
        return Err(DispatchError::GenerationMismatch);
    }
    if request.feature_plan.tier == ExecutionTier::CpuOnly
        || (request.feature_plan.use_f16 && !request.capabilities.advertised.f16)
        || (request.feature_plan.use_subgroups && !request.capabilities.advertised.subgroups)
        || (request.feature_plan.use_float_atomics
            && !request.capabilities.advertised.float_atomics)
    {
        return Err(DispatchError::FeatureUnavailable);
    }
    let reflection = &request.entry.reflection;
    if reflection.entry_point != request.entry.identity.entry_point_id
        || !reflection.stage.eq_ignore_ascii_case("compute")
        || reflection.workgroup_size != POINT_WORKGROUP_SIZE
        || u64::try_from(reflection.bindings.len()).unwrap_or(u64::MAX)
            > u64::from(request.capabilities.limits.max_bindings)
    {
        return Err(DispatchError::ReflectionDrift);
    }
    if !request.entry.identity.owner_operation_ids.is_empty()
        && !request
            .entry
            .identity
            .owner_operation_ids
            .iter()
            .any(|owner| owner == request.operation_id)
    {
        return Err(DispatchError::OperationOwnerMismatch);
    }
    request.parity.validate(reflection)?;
    for transfer in &request.transfers {
        if transfer.generation != request.generation {
            return Err(DispatchError::GenerationMismatch);
        }
    }
    Ok(())
}

fn validate_bindings(
    reflection: &ShaderReflection,
    resources: &[BindingResource],
    generation: DeviceGeneration,
    limits: LimitEnvelope,
) -> Result<Vec<BindingResource>, DispatchError> {
    if resources.len() != reflection.bindings.len() {
        return Err(DispatchError::BindingCountMismatch);
    }
    let mut sorted = resources.to_vec();
    sorted.sort_by_key(|resource| (resource.id.kind, resource.id.index));
    let mut validated = Vec::with_capacity(reflection.bindings.len());
    for expected in &reflection.bindings {
        let resource = resources
            .iter()
            .find(|resource| {
                resource.group == expected.group && resource.binding == expected.binding
            })
            .ok_or(DispatchError::MissingBinding(expected.binding))?;
        resource.validate(expected, generation)?;
        if resource.class.size_bytes > limits.max_buffer_bytes
            || resource.size > limits.max_storage_buffer_bytes
        {
            return Err(DispatchError::BindingLimit(expected.binding));
        }
        validated.push(resource.clone());
    }
    Ok(validated)
}

fn identity_digest(
    request: &PrepareRequest<'_>,
    parameters: &ParameterBlock,
    bindings: &[BindingResource],
    grid: GridPlan,
) -> [u8; 32] {
    let mut bytes = Vec::new();
    push_text(&mut bytes, request.entry.cache_key());
    push_text(&mut bytes, request.operation_id);
    push_text(&mut bytes, request.instance_id);
    bytes.extend_from_slice(&request.implementation_version.to_le_bytes());
    bytes.extend_from_slice(&request.generation.value().to_le_bytes());
    bytes.extend_from_slice(&parameters.identity);
    bytes.extend_from_slice(&binding_bytes(bindings));
    bytes.extend_from_slice(
        &grid
            .logical_extent
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>(),
    );
    bytes.extend_from_slice(
        &grid
            .workgroups
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>(),
    );
    bytes.extend_from_slice(&request.region.roi.x().to_le_bytes());
    bytes.extend_from_slice(&request.region.roi.y().to_le_bytes());
    bytes.extend_from_slice(&request.region.roi.width().to_le_bytes());
    bytes.extend_from_slice(&request.region.roi.height().to_le_bytes());
    digest(&[b"kernel", &bytes])
}

fn binding_bytes(bindings: &[BindingResource]) -> Vec<u8> {
    bindings.iter().fold(Vec::new(), |mut bytes, binding| {
        bytes.extend_from_slice(&binding.id.index.to_le_bytes());
        bytes.extend_from_slice(&binding.group.to_le_bytes());
        bytes.extend_from_slice(&binding.binding.to_le_bytes());
        bytes.extend_from_slice(&binding.offset.to_le_bytes());
        bytes.extend_from_slice(&binding.size.to_le_bytes());
        bytes.extend_from_slice(&binding.usage.to_le_bytes());
        bytes.extend_from_slice(&binding.class.size_bytes.to_le_bytes());
        bytes.extend_from_slice(&binding.class.alignment.to_le_bytes());
        bytes
    })
}

fn push_text(output: &mut Vec<u8>, text: impl AsRef<[u8]>) {
    let text = text.as_ref();
    output.extend_from_slice(&(text.len() as u64).to_le_bytes());
    output.extend_from_slice(text);
}

pub(super) fn digest(parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().into()
}

fn hex(bytes: [u8; 32]) -> String {
    let mut output = String::with_capacity(64);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptStatus {
    Encoded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodingReceipt {
    pub identity: [u8; 32],
    pub status: ReceiptStatus,
    pub command_count: usize,
    pub submitted: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchFailure {
    pub error: DispatchError,
    pub receipt: Box<EncodingReceipt>,
}

impl DispatchFailure {
    pub(super) fn failed(identity: [u8; 32], error: DispatchError) -> Self {
        Self {
            receipt: Box::new(EncodingReceipt {
                identity,
                status: ReceiptStatus::Failed,
                command_count: 0,
                submitted: false,
                error: Some(error.to_string()),
            }),
            error,
        }
    }
    pub(super) fn cancelled(identity: [u8; 32]) -> Self {
        Self {
            error: DispatchError::Cancelled,
            receipt: Box::new(EncodingReceipt {
                identity,
                status: ReceiptStatus::Cancelled,
                command_count: 0,
                submitted: false,
                error: Some(DispatchError::Cancelled.to_string()),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchError {
    ArithmeticOverflow,
    BindingBounds(u32),
    BindingCountMismatch,
    BindingLimit(u32),
    BindingTypeMismatch(u32),
    Cancelled,
    CpuRequired,
    DeviceUnavailable,
    DuplicateParameter(String),
    FeatureUnavailable,
    GenerationMismatch,
    GridOverflow,
    GridReflectionMismatch,
    GridUndercoverage,
    InvalidGrid,
    InvalidIdentity,
    InvalidParameterLayout(String),
    InvalidRegion,
    InvalidTile,
    MissingBinding(u32),
    MissingParameter(String),
    OperationOwnerMismatch,
    OverlappingParameters(String),
    PaddingParameter(String),
    ParameterType {
        name: String,
        expected: String,
        actual: String,
    },
    ParityMismatch,
    ParametersWithoutUniform,
    ReflectionDrift,
    RegionOutsideTile,
    RegionOutOfBounds,
    UnknownParameter(String),
    UnsupportedBinding(u32),
    UsageMismatch(u32),
    WorkgroupLimit,
}

impl fmt::Display for DispatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GPU dispatch validation failed")
    }
}
