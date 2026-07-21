use super::*;
use crate::shader::{
    BindingReflection, BindingResourceKind, FeaturePlan, NumericalClass, NumericalMetadata,
    ParameterReflection, ShaderEntry, ShaderIdentity, ShaderReflection, SourceSpanAlias,
};
use crate::{
    AdapterIdentity, Backend, DeviceGeneration, ExecutionTier, FaultState, GpuCapabilitySnapshot,
    GpuFeaturePlan, LimitEnvelope, ResourceClass, ResourceId, ResourceKind,
};
use rusttable_image::Roi;

const TEST_POINT_WORKGROUP_SIZE: [u32; 3] = [256, 1, 1];
const TEST_STORAGE_USAGE: u64 = 1;
const TEST_UNIFORM_USAGE: u64 = 2;

fn reflection() -> ShaderReflection {
    ShaderReflection {
        schema: "rusttable.reflection.v1".to_owned(),
        entry_point: "copy".to_owned(),
        stage: "Compute".to_owned(),
        bindings: vec![
            BindingReflection {
                group: 0,
                binding: 0,
                name: "input".to_owned(),
                resource: BindingResourceKind::StorageBuffer,
                access: "read".to_owned(),
                address_space: "storage".to_owned(),
                type_description: "array<vec4<f32>>".to_owned(),
                minimum_binding_size: 16,
                dynamic_offset: false,
                dynamic_offset_alignment: 256,
                format: None,
                dimension: None,
                source: SourceSpanAlias {
                    source_alias: "test".to_owned(),
                    line: 1,
                    column: 1,
                },
            },
            BindingReflection {
                group: 0,
                binding: 1,
                name: "output".to_owned(),
                resource: BindingResourceKind::StorageBuffer,
                access: "read_write".to_owned(),
                address_space: "storage".to_owned(),
                type_description: "array<vec4<f32>>".to_owned(),
                minimum_binding_size: 16,
                dynamic_offset: false,
                dynamic_offset_alignment: 256,
                format: None,
                dimension: None,
                source: SourceSpanAlias {
                    source_alias: "test".to_owned(),
                    line: 1,
                    column: 1,
                },
            },
            BindingReflection {
                group: 0,
                binding: 2,
                name: "params".to_owned(),
                resource: BindingResourceKind::UniformBuffer,
                access: "read".to_owned(),
                address_space: "uniform".to_owned(),
                type_description: "struct".to_owned(),
                minimum_binding_size: 16,
                dynamic_offset: false,
                dynamic_offset_alignment: 256,
                format: None,
                dimension: None,
                source: SourceSpanAlias {
                    source_alias: "test".to_owned(),
                    line: 1,
                    column: 1,
                },
            },
        ],
        parameters: vec![ParameterReflection {
            name: "pixel_count".to_owned(),
            scalar_type: "u32".to_owned(),
            offset: 0,
            size: 4,
        }],
        overrides: Vec::new(),
        workgroup_size: TEST_POINT_WORKGROUP_SIZE,
        required_capabilities: Vec::new(),
        source_spans: Vec::new(),
        numerical: NumericalMetadata {
            uses_f32: true,
            uses_f16: false,
            contraction_assumption: "none".to_owned(),
            transcendental_operations: Vec::new(),
            texture_filtering: false,
            sampling: false,
            atomics: false,
            reductions: false,
            subnormal_policy: "preserve-f32".to_owned(),
            non_finite_policy: "reject".to_owned(),
            schema_3_tolerance_class: "PointF32".to_owned(),
            canonical_cpu_reference: "test.cpu".to_owned(),
        },
    }
}

fn entry() -> ShaderEntry {
    let reflection = reflection();
    ShaderEntry {
        identity: ShaderIdentity {
            program_id: "test".to_owned(),
            program_version: 1,
            entry_point_id: "copy".to_owned(),
            entry_point_version: 1,
            source_tree_hash: "source".to_owned(),
            generated_wgsl_hash: "wgsl".to_owned(),
            reflection_schema: reflection.schema.clone(),
            numerical_class: NumericalClass::F32Point,
            feature_plan: FeaturePlan::CoreCompute,
            owner_operation_ids: Vec::new(),
            owner_kernel_ids: vec!["test.kernel".to_owned()],
            canonical_cpu_reference: "test.cpu".to_owned(),
            implementation_version: 1,
        },
        source_alias: "test".to_owned(),
        expanded_source: "@compute fn copy() {}".to_owned(),
        reflection,
    }
}

fn capability() -> GpuCapabilitySnapshot {
    let adapter = AdapterIdentity::new(Backend::Vulkan, 1, 2, "test-device");
    GpuCapabilitySnapshot {
        schema_version: 1,
        generation: 7,
        backend: Backend::Vulkan,
        adapter: Some(adapter),
        limits: LimitEnvelope {
            max_storage_buffer_bytes: 4096,
            max_buffer_bytes: 4096,
            max_bindings: 4,
            max_workgroups_per_dimension: 64,
            supports_rgba16f_storage: false,
            supports_r32f_storage: false,
        },
        advertised: crate::AdvertisedFeatures::none(),
        probes: crate::ProbeLedger {
            compute_storage_buffers: true,
            rgba16f_sampled_attachment: true,
            r32f_storage_or_buffer_path: true,
            binding_alignment: true,
            copy_map_readback: true,
            optional: crate::AdvertisedFeatures::none(),
        },
        tier: ExecutionTier::CoreCompute,
        state: FaultState::Healthy,
    }
}

fn request() -> PrepareRequest<'static> {
    let entry = Box::leak(Box::new(entry()));
    let capability = Box::leak(Box::new(capability()));
    let input_id = ResourceId {
        generation: DeviceGeneration::new(7),
        index: 0,
        kind: ResourceKind::Buffer,
    };
    let output_id = ResourceId {
        generation: DeviceGeneration::new(7),
        index: 1,
        kind: ResourceKind::Buffer,
    };
    let input_class = ResourceClass::buffer(DeviceGeneration::new(7), 1024, TEST_STORAGE_USAGE)
        .with_alignment(256);
    let output_class = ResourceClass::buffer(DeviceGeneration::new(7), 1024, TEST_STORAGE_USAGE)
        .with_alignment(256);
    let input = BindingResource::from_reflection(
        &entry.reflection.bindings[0],
        input_id,
        input_class,
        0,
        1024,
    );
    let output = BindingResource::from_reflection(
        &entry.reflection.bindings[1],
        output_id,
        output_class,
        0,
        1024,
    );
    PrepareRequest {
        operation_id: "test.operation",
        instance_id: "instance",
        implementation_version: 1,
        entry,
        capabilities: capability,
        generation: DeviceGeneration::new(7),
        feature_plan: GpuFeaturePlan {
            tier: ExecutionTier::CoreCompute,
            use_f16: false,
            use_subgroups: false,
            use_float_atomics: false,
        },
        parameters: TypedParameters::new().with("pixel_count", ScalarValue::U32(256)),
        bindings: vec![
            input,
            output,
            BindingResource::from_reflection(
                &entry.reflection.bindings[2],
                ResourceId {
                    generation: DeviceGeneration::new(7),
                    index: 2,
                    kind: ResourceKind::Buffer,
                },
                ResourceClass::buffer(DeviceGeneration::new(7), 256, TEST_UNIFORM_USAGE)
                    .with_alignment(256),
                0,
                256,
            ),
        ],
        region: DispatchRegion::new(
            256,
            1,
            Roi::new(0, 0, 256, 1).expect("roi"),
            Tile::new(0, 0, 256, 1).expect("tile"),
        )
        .expect("region"),
        grid: None,
        transfers: Vec::new(),
        parity: ParityContract {
            cpu_reference: "test.cpu".to_owned(),
            tolerance_class: "PointF32".to_owned(),
            absolute: 0.001,
            relative: 0.001,
        },
        cancellation: CancellationToken::new(),
    }
}

#[test]
fn parameter_packing_is_typed_and_zero_padded() {
    let prepared = PreparedGpuKernel::prepare(request()).expect("prepared");
    assert_eq!(
        &prepared.parameter_block().bytes()[0..4],
        &256_u32.to_le_bytes()
    );
    assert!(
        prepared.parameter_block().bytes()[4..]
            .iter()
            .all(|byte| *byte == 0)
    );
}

#[test]
fn identities_are_deterministic_and_change_with_parameters() {
    let first = PreparedGpuKernel::prepare(request()).expect("first");
    let second = PreparedGpuKernel::prepare(request()).expect("second");
    assert_eq!(first.identity_digest(), second.identity_digest());
    let mut changed = request();
    changed.parameters = TypedParameters::new().with("pixel_count", ScalarValue::U32(257));
    let third = PreparedGpuKernel::prepare(changed).expect("third");
    assert_ne!(first.identity_digest(), third.identity_digest());
}

#[test]
fn invalid_grid_is_rejected_before_encoding() {
    let request = request();
    let error = GridPlan::checked(
        [257, 1, 1],
        TEST_POINT_WORKGROUP_SIZE,
        [1, 1, 1],
        request.capabilities.limits,
    )
    .expect_err("one workgroup undercovers 257 invocations");
    assert_eq!(error, DispatchError::GridUndercoverage);
}

#[test]
fn encoding_failure_does_not_append_commands() {
    let prepared = PreparedGpuKernel::prepare(request()).expect("prepared");
    let mut encoder = CommandEncoder::new();
    encoder.reject_next();
    let failure = prepared.encode(&mut encoder).expect_err("rejected");
    assert_eq!(failure.receipt.status, ReceiptStatus::Failed);
    assert!(encoder.commands().is_empty());
    assert!(!failure.receipt.submitted);
}

#[test]
fn cancellation_produces_a_non_submitting_receipt() {
    let request = request();
    request.cancellation.cancel();
    let failure = PreparedGpuKernel::prepare(request).expect_err("cancelled");
    assert_eq!(failure.receipt.status, ReceiptStatus::Cancelled);
    assert!(!failure.receipt.submitted);
}

#[test]
fn batch_encoding_is_all_or_nothing() {
    let first = PreparedGpuKernel::prepare(request()).expect("first");
    let second = PreparedGpuKernel::prepare(request()).expect("second");
    second.cancellation.cancel();
    let batch = DispatchBatch::new(vec![first, second]).expect("batch");
    let mut encoder = CommandEncoder::new();
    let failure = batch.encode(&mut encoder).expect_err("cancelled");
    assert_eq!(failure.receipt.command_count, 0);
    assert!(encoder.commands().is_empty());
}
