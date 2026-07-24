use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::sync::OnceLock;

use rusttable_core::numerics::{
    CompilerBaseline, ConversionPolicy, ConversionRange, FloatDomainPolicy, FmaPolicy,
    ImplementationFamily, ImplementationNumerics, NonFinitePolicy, NumericalContract,
    ReductionPolicy, RoundingPolicy, SubnormalPolicy, ToleranceClass, TranscendentalPolicy,
};
use sha2::{Digest, Sha256};

use super::model::{
    BindingReflection, BindingResourceKind, FeaturePlan, NumericalClass, NumericalMetadata,
    SHADER_SCHEMA, ShaderEntry, ShaderError, ShaderIdentity, ShaderManifest,
};
use super::source::SourceCatalog;
use super::validate::validate_and_reflect;

const POINT_SOURCE: &str = "shaders/point.wgsl";
const BILATERAL_SOURCE: &str = "shaders/bilateral.wgsl";

#[derive(Debug, Clone, Copy)]
struct EntrySpec {
    id: &'static str,
    owner_operation: Option<&'static str>,
    owner_kernel: &'static str,
    cpu_reference: &'static str,
    transcendental: &'static [&'static str],
}

const ENTRY_SPECS: &[EntrySpec] = &[
    EntrySpec {
        id: "transfer_decode",
        owner_operation: None,
        owner_kernel: "rusttable.kernel.transfer-decode",
        cpu_reference: "infrastructure.none",
        transcendental: &["pow"],
    },
    EntrySpec {
        id: "transfer_encode",
        owner_operation: None,
        owner_kernel: "rusttable.kernel.transfer-encode",
        cpu_reference: "infrastructure.none",
        transcendental: &["pow"],
    },
    EntrySpec {
        id: "exposure",
        owner_operation: Some("rusttable.exposure"),
        owner_kernel: "darktable.basic.exposure",
        cpu_reference: "rusttable.cpu.exposure",
        transcendental: &["exp2"],
    },
    EntrySpec {
        id: "basicadj",
        owner_operation: Some("rusttable.basicadj"),
        owner_kernel: "darktable.basicadj",
        cpu_reference: "rusttable.cpu.basicadj",
        transcendental: &["pow", "log"],
    },
    EntrySpec {
        id: "linear_offset",
        owner_operation: None,
        owner_kernel: "rusttable.kernel.linear-offset",
        cpu_reference: "infrastructure.none",
        transcendental: &[],
    },
    EntrySpec {
        id: "rgb_gain",
        owner_operation: None,
        owner_kernel: "rusttable.kernel.rgb-gain",
        cpu_reference: "infrastructure.none",
        transcendental: &[],
    },
    EntrySpec {
        id: "copy",
        owner_operation: None,
        owner_kernel: "rusttable.kernel.copy",
        cpu_reference: "infrastructure.none",
        transcendental: &[],
    },
    EntrySpec {
        id: "probe",
        owner_operation: None,
        owner_kernel: "rusttable.kernel.probe",
        cpu_reference: "infrastructure.none",
        transcendental: &[],
    },
];

#[derive(Debug, Clone, Copy)]
pub(crate) struct BindingContract {
    pub binding: u32,
    pub name: &'static str,
    pub storage: bool,
    pub access: &'static str,
    pub minimum_binding_size: u32,
    pub type_description: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct BilateralEntryContract {
    pub entry_point: &'static str,
    pub workgroup_size: [u32; 3],
    pub bindings: &'static [BindingContract],
}

const POINT_COMMON_BINDINGS: &[BindingContract] = &[
    BindingContract {
        binding: 0,
        name: "input_pixels",
        storage: true,
        access: "read",
        minimum_binding_size: 16,
        type_description: "array<vec4<f32>, runtime>",
    },
    BindingContract {
        binding: 1,
        name: "output_pixels",
        storage: true,
        access: "read_write",
        minimum_binding_size: 16,
        type_description: "array<vec4<f32>, runtime>",
    },
    BindingContract {
        binding: 2,
        name: "params",
        storage: false,
        access: "read",
        minimum_binding_size: 64,
        type_description: "struct",
    },
];

const POINT_BASICADJ_BINDINGS: &[BindingContract] = &[
    POINT_COMMON_BINDINGS[0],
    POINT_COMMON_BINDINGS[1],
    POINT_COMMON_BINDINGS[2],
    BindingContract {
        binding: 3,
        name: "basic_params",
        storage: false,
        access: "read",
        minimum_binding_size: 48,
        type_description: "struct",
    },
];

pub(crate) const BILATERAL_ENTRY_CONTRACTS: &[BilateralEntryContract] = &[
    BilateralEntryContract {
        entry_point: "zero",
        workgroup_size: [16, 16, 1],
        bindings: &[
            BindingContract {
                binding: 0,
                name: "zero_grid",
                storage: true,
                access: "read_write",
                minimum_binding_size: 4,
                type_description: "array<atomic<u32>, runtime>",
            },
            BindingContract {
                binding: 1,
                name: "params",
                storage: false,
                access: "read",
                minimum_binding_size: 64,
                type_description: "struct",
            },
        ],
    },
    BilateralEntryContract {
        entry_point: "splat",
        workgroup_size: [16, 16, 1],
        bindings: &[
            BindingContract {
                binding: 1,
                name: "params",
                storage: false,
                access: "read",
                minimum_binding_size: 64,
                type_description: "struct",
            },
            BindingContract {
                binding: 2,
                name: "splat_input",
                storage: true,
                access: "read",
                minimum_binding_size: 4,
                type_description: "array<f32, runtime>",
            },
            BindingContract {
                binding: 3,
                name: "splat_grid",
                storage: true,
                access: "read_write",
                minimum_binding_size: 4,
                type_description: "array<atomic<u32>, runtime>",
            },
        ],
    },
    BilateralEntryContract {
        entry_point: "blur_line",
        workgroup_size: [8, 8, 1],
        bindings: &[
            BindingContract {
                binding: 1,
                name: "params",
                storage: false,
                access: "read",
                minimum_binding_size: 64,
                type_description: "struct",
            },
            BindingContract {
                binding: 4,
                name: "blur_input",
                storage: true,
                access: "read",
                minimum_binding_size: 4,
                type_description: "array<f32, runtime>",
            },
            BindingContract {
                binding: 5,
                name: "blur_output",
                storage: true,
                access: "read_write",
                minimum_binding_size: 4,
                type_description: "array<f32, runtime>",
            },
        ],
    },
    BilateralEntryContract {
        entry_point: "blur_line_z",
        workgroup_size: [8, 8, 1],
        bindings: &[
            BindingContract {
                binding: 1,
                name: "params",
                storage: false,
                access: "read",
                minimum_binding_size: 64,
                type_description: "struct",
            },
            BindingContract {
                binding: 4,
                name: "blur_input",
                storage: true,
                access: "read",
                minimum_binding_size: 4,
                type_description: "array<f32, runtime>",
            },
            BindingContract {
                binding: 5,
                name: "blur_output",
                storage: true,
                access: "read_write",
                minimum_binding_size: 4,
                type_description: "array<f32, runtime>",
            },
        ],
    },
    BilateralEntryContract {
        entry_point: "slice",
        workgroup_size: [16, 16, 1],
        bindings: &[
            BindingContract {
                binding: 1,
                name: "params",
                storage: false,
                access: "read",
                minimum_binding_size: 64,
                type_description: "struct",
            },
            BindingContract {
                binding: 6,
                name: "slice_input",
                storage: true,
                access: "read",
                minimum_binding_size: 4,
                type_description: "array<f32, runtime>",
            },
            BindingContract {
                binding: 7,
                name: "slice_target",
                storage: true,
                access: "read",
                minimum_binding_size: 4,
                type_description: "array<f32, runtime>",
            },
            BindingContract {
                binding: 8,
                name: "slice_output",
                storage: true,
                access: "read_write",
                minimum_binding_size: 4,
                type_description: "array<f32, runtime>",
            },
            BindingContract {
                binding: 9,
                name: "slice_grid",
                storage: true,
                access: "read",
                minimum_binding_size: 4,
                type_description: "array<f32, runtime>",
            },
        ],
    },
    BilateralEntryContract {
        entry_point: "slice_to_output",
        workgroup_size: [16, 16, 1],
        bindings: &[
            BindingContract {
                binding: 1,
                name: "params",
                storage: false,
                access: "read",
                minimum_binding_size: 64,
                type_description: "struct",
            },
            BindingContract {
                binding: 6,
                name: "slice_input",
                storage: true,
                access: "read",
                minimum_binding_size: 4,
                type_description: "array<f32, runtime>",
            },
            BindingContract {
                binding: 7,
                name: "slice_target",
                storage: true,
                access: "read",
                minimum_binding_size: 4,
                type_description: "array<f32, runtime>",
            },
            BindingContract {
                binding: 8,
                name: "slice_output",
                storage: true,
                access: "read_write",
                minimum_binding_size: 4,
                type_description: "array<f32, runtime>",
            },
            BindingContract {
                binding: 9,
                name: "slice_grid",
                storage: true,
                access: "read",
                minimum_binding_size: 4,
                type_description: "array<f32, runtime>",
            },
        ],
    },
];

fn point_bindings_match(entry_point: &str, actual: &[BindingReflection]) -> bool {
    let expected = if entry_point == "basicadj" {
        POINT_BASICADJ_BINDINGS
    } else {
        POINT_COMMON_BINDINGS
    };
    bindings_match(actual, expected)
}

fn bilateral_bindings_match(entry_point: &str, actual: &[BindingReflection]) -> bool {
    BILATERAL_ENTRY_CONTRACTS
        .iter()
        .find(|contract| contract.entry_point == entry_point)
        .is_some_and(|contract| bindings_match(actual, contract.bindings))
}

fn bindings_match(actual: &[BindingReflection], expected: &[BindingContract]) -> bool {
    actual.len() == expected.len()
        && actual.iter().zip(expected).all(|(binding, expected)| {
            let resource_matches = if expected.storage {
                binding.resource == BindingResourceKind::StorageBuffer
            } else {
                binding.resource == BindingResourceKind::UniformBuffer
            };
            binding.group == 0
                && binding.binding == expected.binding
                && binding.name == expected.name
                && resource_matches
                && binding.access == expected.access
                && binding.address_space
                    == if expected.storage {
                        "storage"
                    } else {
                        "uniform"
                    }
                && binding.minimum_binding_size == expected.minimum_binding_size
                && binding.type_description == expected.type_description
                && !binding.dynamic_offset
                && binding.format.is_none()
                && binding.dimension.is_none()
        })
}

fn generated_constant_name(entry: &ShaderEntry) -> String {
    let identity = &entry.identity;
    let stem = if identity.program_id == "rusttable.point" {
        identity.entry_point_id.clone()
    } else {
        format!(
            "{}_{}",
            identity
                .program_id
                .strip_prefix("rusttable.")
                .unwrap_or(&identity.program_id),
            identity.entry_point_id
        )
    };
    stem.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct ShaderRegistry {
    entries: Vec<ShaderEntry>,
}

impl ShaderRegistry {
    pub fn try_checked_in() -> Result<Self, ShaderError> {
        let catalog = SourceCatalog::checked_in();
        let mut entries = Vec::new();
        let mut identities = BTreeSet::new();
        let substitutions = BTreeMap::from([("WORKGROUP_SIZE".to_owned(), "256u".to_owned())]);
        for spec in ENTRY_SPECS {
            let expanded = catalog.expand(POINT_SOURCE, &substitutions)?;
            let (contract, numerical) = numerical_metadata(spec);
            let reflection = validate_and_reflect(
                POINT_SOURCE,
                &expanded.text,
                &expanded.line_aliases,
                spec.id,
                numerical,
            )?;
            if reflection.workgroup_size != [256, 1, 1] {
                return Err(ShaderError::Reflection(spec.id.to_owned()));
            }
            if !point_bindings_match(spec.id, &reflection.bindings) {
                return Err(ShaderError::Reflection(format!(
                    "{} binding contract",
                    spec.id
                )));
            }
            let source_tree_hash = catalog.source_tree_hash(POINT_SOURCE)?;
            let generated_wgsl_hash = digest(&expanded.text);
            let implementation_id = format!("rusttable.point.{}", spec.id);
            let implementation_numerics = ImplementationNumerics::new(
                &implementation_id,
                spec.cpu_reference,
                &generated_wgsl_hash,
                ImplementationFamily::Gpu,
                CompilerBaseline::BackendToolchain,
                ToleranceClass::Pointwise,
                contract,
            )
            .map_err(|error| ShaderError::Reflection(error.to_string()))?;
            let identity = ShaderIdentity {
                program_id: "rusttable.point".to_owned(),
                program_version: 1,
                entry_point_id: spec.id.to_owned(),
                entry_point_version: 2,
                source_tree_hash,
                generated_wgsl_hash,
                reflection_schema: reflection.schema.clone(),
                numerical_class: NumericalClass::F32Point,
                feature_plan: FeaturePlan::CoreCompute,
                owner_operation_ids: spec
                    .owner_operation
                    .map_or_else(Vec::new, |value| vec![value.to_owned()]),
                owner_kernel_ids: vec![spec.owner_kernel.to_owned()],
                canonical_cpu_reference: spec.cpu_reference.to_owned(),
                implementation_version: 1,
                implementation_numerics,
            };
            let identity_name = identity.entry_id().stable_name();
            if !identities.insert(identity_name.clone()) {
                return Err(ShaderError::DuplicateIdentity(identity_name));
            }
            if identity.owner_operation_ids.is_empty() && identity.owner_kernel_ids.is_empty() {
                return Err(ShaderError::MissingOwner(spec.id.to_owned()));
            }
            if reflection.numerical.schema_3_tolerance_class.is_empty() {
                return Err(ShaderError::MissingTolerance(spec.id.to_owned()));
            }
            if identity.implementation_numerics.contract() != reflection.numerical.contract
                || identity.implementation_numerics.tolerance() != reflection.numerical.tolerance
                || identity.implementation_numerics.scalar_reference_id()
                    != reflection.numerical.canonical_cpu_reference
            {
                return Err(ShaderError::Reflection(format!(
                    "{} numerical registration",
                    spec.id
                )));
            }
            entries.push(ShaderEntry {
                identity,
                source_alias: POINT_SOURCE.to_owned(),
                expanded_source: expanded.text,
                reflection,
            });
        }
        let expanded = catalog.expand(BILATERAL_SOURCE, &BTreeMap::new())?;
        let source_tree_hash = catalog.source_tree_hash(BILATERAL_SOURCE)?;
        for entry_contract in BILATERAL_ENTRY_CONTRACTS {
            let entry_point = entry_contract.entry_point;
            let (contract, numerical) = bilateral_numerical_metadata(entry_point);
            let reflection = validate_and_reflect(
                BILATERAL_SOURCE,
                &expanded.text,
                &expanded.line_aliases,
                entry_point,
                numerical,
            )?;
            if reflection.workgroup_size != entry_contract.workgroup_size {
                return Err(ShaderError::Reflection(format!("{entry_point} workgroup")));
            }
            if !bilateral_bindings_match(entry_point, &reflection.bindings) {
                return Err(ShaderError::Reflection(format!(
                    "{entry_point} binding contract"
                )));
            }
            let generated_wgsl_hash = digest(&expanded.text);
            let implementation_id = format!("rusttable.bilateral.{entry_point}");
            let implementation_numerics = ImplementationNumerics::new(
                &implementation_id,
                "rusttable.cpu.bilateral",
                &generated_wgsl_hash,
                ImplementationFamily::Gpu,
                CompilerBaseline::BackendToolchain,
                reflection.numerical.tolerance,
                contract,
            )
            .map_err(|error| ShaderError::Reflection(error.to_string()))?;
            let identity = ShaderIdentity {
                program_id: "rusttable.bilateral".to_owned(),
                program_version: 1,
                entry_point_id: entry_point.to_owned(),
                entry_point_version: 1,
                source_tree_hash: source_tree_hash.clone(),
                generated_wgsl_hash,
                reflection_schema: reflection.schema.clone(),
                numerical_class: NumericalClass::F32Neighborhood,
                feature_plan: FeaturePlan::CoreCompute,
                owner_operation_ids: vec!["rusttable.shadhi".to_owned()],
                owner_kernel_ids: vec![format!("darktable.common.bilateral.{entry_point}")],
                canonical_cpu_reference: "rusttable.cpu.bilateral".to_owned(),
                implementation_version: 1,
                implementation_numerics,
            };
            let identity_name = identity.entry_id().stable_name();
            if !identities.insert(identity_name.clone()) {
                return Err(ShaderError::DuplicateIdentity(identity_name));
            }
            if identity.implementation_numerics.contract() != reflection.numerical.contract
                || identity.implementation_numerics.tolerance() != reflection.numerical.tolerance
                || identity.implementation_numerics.scalar_reference_id()
                    != reflection.numerical.canonical_cpu_reference
            {
                return Err(ShaderError::Reflection(format!(
                    "{entry_point} numerical registration"
                )));
            }
            entries.push(ShaderEntry {
                identity,
                source_alias: BILATERAL_SOURCE.to_owned(),
                expanded_source: expanded.text.clone(),
                reflection,
            });
        }
        entries.sort_by_key(super::model::ShaderEntry::id);
        Ok(Self { entries })
    }

    #[must_use]
    /// # Panics
    ///
    /// Panics only when a checked-in source or generated manifest fails validation.
    pub fn checked_in() -> &'static Self {
        static REGISTRY: OnceLock<ShaderRegistry> = OnceLock::new();
        REGISTRY.get_or_init(|| Self::try_checked_in().expect("checked-in shaders must validate"))
    }

    #[must_use]
    pub fn entries(&self) -> &[ShaderEntry] {
        &self.entries
    }

    #[must_use]
    pub fn find(&self, program_id: &str, entry_point_id: &str) -> Option<&ShaderEntry> {
        self.entries.iter().find(|entry| {
            entry.identity.program_id == program_id
                && entry.identity.entry_point_id == entry_point_id
        })
    }

    #[must_use]
    pub fn manifest(&self) -> ShaderManifest {
        let mut text = format!(
            "schema = \"{SHADER_SCHEMA}\"\nreflection_schema = \"{}\"\n\n",
            super::model::REFLECTION_SCHEMA
        );
        for entry in &self.entries {
            let identity = &entry.identity;
            let _ = writeln!(
                text,
                "[[shader]]\nid = \"{}\"\nprogram_version = {}\nentry_point_version = {}\nimplementation_version = {}\nsource_alias = \"{}\"\nsource_tree_hash = \"{}\"\ngenerated_wgsl_hash = \"{}\"\nfeature_plan = \"{:?}\"\nnumerical_class = \"{:?}\"\ncanonical_cpu_reference = \"{}\"\nowner_operations = {:?}\nowner_kernels = {:?}\nworkgroup_size = {:?}\nuses_f32 = {}\nuses_f16 = {}\nfloat_domain = \"{:?}\"\nnon_finite_policy = \"{:?}\"\nsubnormal_policy = \"{:?}\"\nfma_policy = \"{:?}\"\nreduction_policy = \"{:?}\"\ntranscendental_policy = \"{:?}\"\ntranscendental_operations = {:?}\ntexture_filtering = {}\nsampling = {}\natomics = {}\nreductions = {}\ntolerance_class = \"{}\"\nnumerical_contract_id = \"{}\"\n",
                entry.id().stable_name(),
                identity.program_version,
                identity.entry_point_version,
                identity.implementation_version,
                entry.source_alias,
                identity.source_tree_hash,
                identity.generated_wgsl_hash,
                identity.feature_plan,
                identity.numerical_class,
                identity.canonical_cpu_reference,
                identity.owner_operation_ids,
                identity.owner_kernel_ids,
                entry.reflection.workgroup_size,
                entry.reflection.numerical.uses_f32,
                entry.reflection.numerical.uses_f16,
                entry.reflection.numerical.contract.float_domain,
                entry.reflection.numerical.contract.non_finite,
                entry.reflection.numerical.contract.subnormal,
                entry.reflection.numerical.contract.fma,
                entry.reflection.numerical.contract.reduction,
                entry.reflection.numerical.contract.transcendental,
                entry.reflection.numerical.transcendental_operations,
                entry.reflection.numerical.texture_filtering,
                entry.reflection.numerical.sampling,
                entry.reflection.numerical.atomics,
                entry.reflection.numerical.reductions,
                entry.reflection.numerical.tolerance.as_str(),
                entry.reflection.numerical.contract.stable_id(),
            );
            for binding in &entry.reflection.bindings {
                let _ = writeln!(
                    text,
                    "[[shader.binding]]\nshader = \"{}\"\ngroup = {}\nbinding = {}\nname = \"{}\"\nresource = \"{:?}\"\naccess = \"{}\"\naddress_space = \"{}\"\ntype = \"{}\"\nminimum_binding_size = {}\ndynamic_offset = {}\ndynamic_offset_alignment = {}\nsource_alias = \"{}\"\nsource_line = {}\nsource_column = {}\n",
                    entry.id().stable_name(),
                    binding.group,
                    binding.binding,
                    binding.name,
                    binding.resource,
                    binding.access,
                    binding.address_space,
                    binding.type_description,
                    binding.minimum_binding_size,
                    binding.dynamic_offset,
                    binding.dynamic_offset_alignment,
                    binding.source.source_alias,
                    binding.source.line,
                    binding.source.column
                );
            }
        }
        ShaderManifest { text }
    }

    #[must_use]
    /// # Panics
    ///
    /// Panics if a validated registry contains shader identities that collapse
    /// to the same generated Rust constant name.
    pub fn generated_bindings_source(&self) -> String {
        let mut output =
            String::from("// GENERATED FILE: cargo xtask shaders generate; do not hand-edit.\n\n");
        output.push_str(
            "pub const GENERATED_BINDING_SCHEMA: &str = \"rusttable.shader-bindings.v1\";\n",
        );
        let point_params_size = self
            .find("rusttable.point", "exposure")
            .and_then(|entry| {
                entry
                    .reflection
                    .bindings
                    .iter()
                    .find(|binding| binding.binding == 2)
            })
            .map_or(0, |binding| binding.minimum_binding_size);
        let _ = writeln!(
            output,
            "pub const POINT_PARAMS_SIZE: usize = {point_params_size};\n"
        );
        let mut generated_names = BTreeSet::new();
        for entry in &self.entries {
            let name = generated_constant_name(entry);
            assert!(
                generated_names.insert(name.clone()),
                "validated shader identities must generate unique Rust constants"
            );
            let _ = writeln!(
                output,
                "pub const ENTRY_{}_ID: &str = \"{}\";",
                name,
                entry.id().stable_name()
            );
        }
        output.push_str("\n#[derive(Debug, Clone, Copy, PartialEq)]\n#[repr(C)]\npub struct PointParams {\n    pub pixel_count: u32,\n    pub exposure_stops: f32,\n    pub linear_offset: f32,\n    pub gain_red: f32,\n    pub gain_green: f32,\n    pub gain_blue: f32,\n    pub transfer_gamma: f32,\n    pub reserved: [u32; 5],\n}\n\nimpl PointParams {\n    #[must_use]\n    pub fn bytes(self) -> [u8; POINT_PARAMS_SIZE] {\n        let mut bytes = [0u8; POINT_PARAMS_SIZE];\n        let words = [self.pixel_count.to_le_bytes(), self.exposure_stops.to_le_bytes(), self.linear_offset.to_le_bytes(), self.gain_red.to_le_bytes(), self.gain_green.to_le_bytes(), self.gain_blue.to_le_bytes(), self.transfer_gamma.to_le_bytes(), self.reserved[0].to_le_bytes(), self.reserved[1].to_le_bytes(), self.reserved[2].to_le_bytes(), self.reserved[3].to_le_bytes(), self.reserved[4].to_le_bytes()];\n        for (index, word) in words.into_iter().enumerate() { bytes[index * 4..index * 4 + 4].copy_from_slice(&word); }\n        bytes\n    }\n}\n");
        output.truncate(output.find("\n#[derive").unwrap_or(output.len()));
        output
    }

    pub fn verify_checked_in_outputs(&self) -> Result<(), ShaderError> {
        let manifest = include_str!("../../../../architecture/rusttable-shader-manifest.toml");
        if manifest != self.manifest().text {
            return Err(ShaderError::ManifestDrift);
        }
        let generated = include_str!("generated.rs");
        if generated != self.generated_bindings_source() {
            return Err(ShaderError::GeneratedBindingsDrift);
        }
        Ok(())
    }

    #[must_use]
    pub fn point_source(&self) -> &str {
        self.entries
            .iter()
            .find(|entry| entry.identity.program_id == "rusttable.point")
            .map_or("", |entry| &entry.expanded_source)
    }
}

fn numerical_metadata(spec: &EntrySpec) -> (NumericalContract, NumericalMetadata) {
    let contract = NumericalContract {
        float_domain: FloatDomainPolicy::F32,
        non_finite: NonFinitePolicy::Reject,
        subnormal: SubnormalPolicy::BackendDefined,
        fma: FmaPolicy::BackendDefined,
        reduction: ReductionPolicy::None,
        transcendental: if spec.transcendental.is_empty() {
            TranscendentalPolicy::None
        } else {
            TranscendentalPolicy::WgslBackend
        },
        conversion: ConversionPolicy::checked_nearest_even(),
    };
    let metadata = NumericalMetadata {
        uses_f32: true,
        uses_f16: false,
        contraction_assumption: "backend-defined; PointF32 tolerance required".to_owned(),
        transcendental_operations: spec
            .transcendental
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
        texture_filtering: false,
        sampling: false,
        atomics: false,
        reductions: false,
        subnormal_policy: "backend-defined".to_owned(),
        non_finite_policy: "reject-at-host-boundary".to_owned(),
        schema_3_tolerance_class: "PointF32".to_owned(),
        canonical_cpu_reference: spec.cpu_reference.to_owned(),
        contract,
        tolerance: ToleranceClass::Pointwise,
    };
    (contract, metadata)
}

fn bilateral_numerical_metadata(entry_point: &str) -> (NumericalContract, NumericalMetadata) {
    let (reduction, tolerance, atomics, reductions, subnormal, fma) = match entry_point {
        "zero" => (
            ReductionPolicy::None,
            ToleranceClass::Exact,
            true,
            false,
            SubnormalPolicy::Preserve,
            FmaPolicy::SeparateRoundings,
        ),
        "splat" => (
            ReductionPolicy::BackendDefined,
            ToleranceClass::LegacyGpu,
            true,
            true,
            SubnormalPolicy::BackendDefined,
            FmaPolicy::BackendDefined,
        ),
        _ => (
            ReductionPolicy::BackendDefined,
            ToleranceClass::Neighborhood,
            false,
            true,
            SubnormalPolicy::BackendDefined,
            FmaPolicy::BackendDefined,
        ),
    };
    let contract = NumericalContract {
        float_domain: FloatDomainPolicy::F32,
        non_finite: NonFinitePolicy::Reject,
        subnormal,
        fma,
        reduction,
        transcendental: TranscendentalPolicy::None,
        conversion: if matches!(entry_point, "splat" | "slice" | "slice_to_output") {
            ConversionPolicy {
                range: ConversionRange::Clamp,
                rounding: RoundingPolicy::TowardZero,
            }
        } else {
            ConversionPolicy::checked_nearest_even()
        },
    };
    let metadata = NumericalMetadata {
        uses_f32: true,
        uses_f16: false,
        contraction_assumption: if entry_point == "zero" {
            "no floating-point arithmetic; exact zero store".to_owned()
        } else {
            format!("backend-defined; {} tolerance required", tolerance.as_str())
        },
        transcendental_operations: Vec::new(),
        texture_filtering: false,
        sampling: false,
        atomics,
        reductions,
        subnormal_policy: if entry_point == "zero" {
            "preserve".to_owned()
        } else {
            "backend-defined".to_owned()
        },
        non_finite_policy: "reject-at-host-boundary".to_owned(),
        schema_3_tolerance_class: tolerance.as_str().to_owned(),
        canonical_cpu_reference: "rusttable.cpu.bilateral".to_owned(),
        contract,
        tolerance,
    };
    (contract, metadata)
}

fn digest(source: &str) -> String {
    let digest: [u8; 32] = Sha256::digest(source.as_bytes()).into();
    super::model::hex(&digest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_in_registry_has_stable_initial_entries() {
        let registry = ShaderRegistry::try_checked_in().expect("registry");
        assert_eq!(registry.entries().len(), 14);
        let exposure = registry
            .find("rusttable.point", "exposure")
            .expect("point exposure");
        assert_eq!(exposure.reflection.bindings.len(), 3);
        assert_eq!(exposure.identity.entry_point_version, 2);
        assert_eq!(
            registry
                .find("rusttable.point", "basicadj")
                .expect("point basicadj")
                .reflection
                .bindings
                .len(),
            4
        );
        assert_eq!(
            registry
                .find("rusttable.point", "basicadj")
                .expect("point basicadj")
                .identity
                .entry_point_version,
            2
        );
        assert_eq!(exposure.reflection.workgroup_size, [256, 1, 1]);
        assert!(
            registry
                .entries()
                .iter()
                .any(|entry| entry.id().stable_name() == "rusttable.point.exposure")
        );
        for entry in registry
            .entries()
            .iter()
            .filter(|entry| entry.identity.program_id == "rusttable.point")
        {
            assert_eq!(
                entry.identity.implementation_numerics.tolerance(),
                ToleranceClass::Pointwise
            );
            assert!(
                entry
                    .identity
                    .implementation_numerics
                    .contract()
                    .has_backend_defined_behavior()
            );
            assert_eq!(
                entry.identity.implementation_numerics.contract(),
                entry.reflection.numerical.contract
            );
        }
    }

    #[test]
    fn bilateral_splat_records_reduction_numerics_without_breaking_point_source() {
        let registry = ShaderRegistry::try_checked_in().expect("registry");
        for entry_name in [
            "zero",
            "splat",
            "blur_line",
            "blur_line_z",
            "slice",
            "slice_to_output",
        ] {
            let entry = registry
                .find("rusttable.bilateral", entry_name)
                .expect("bilateral entry");
            assert_eq!(
                entry.identity.numerical_class,
                NumericalClass::F32Neighborhood
            );
        }
        let splat = registry
            .find("rusttable.bilateral", "splat")
            .expect("bilateral splat");
        let zero = registry
            .find("rusttable.bilateral", "zero")
            .expect("bilateral zero");
        assert_eq!(
            splat.identity.implementation_numerics.tolerance(),
            ToleranceClass::LegacyGpu
        );
        assert_eq!(
            splat.identity.implementation_numerics.contract().reduction,
            ReductionPolicy::BackendDefined
        );
        assert_eq!(
            splat.identity.implementation_numerics.contract().conversion,
            ConversionPolicy {
                range: ConversionRange::Clamp,
                rounding: RoundingPolicy::TowardZero,
            }
        );
        assert!(splat.reflection.numerical.atomics);
        assert!(splat.reflection.numerical.reductions);
        assert!(zero.reflection.numerical.atomics);
        assert_eq!(
            zero.reflection.bindings[0].type_description,
            "array<atomic<u32>, runtime>"
        );
        let blur = registry
            .find("rusttable.bilateral", "blur_line")
            .expect("bilateral blur");
        assert_eq!(
            blur.identity.implementation_numerics.contract().reduction,
            ReductionPolicy::BackendDefined
        );
        assert_eq!(
            blur.reflection.bindings[1].type_description,
            "array<f32, runtime>"
        );
        let mut drifted = blur.reflection.bindings.clone();
        drifted[1].binding = 6;
        assert!(!bilateral_bindings_match("blur_line", &drifted));
        let mut point_drifted = registry
            .find("rusttable.point", "exposure")
            .expect("point exposure")
            .reflection
            .bindings
            .clone();
        point_drifted[2].minimum_binding_size = 48;
        assert!(!point_bindings_match("exposure", &point_drifted));
        let generated = registry.generated_bindings_source();
        assert!(generated.contains("pub const POINT_PARAMS_SIZE: usize = 64;"));
        assert!(generated.contains(
            "pub const ENTRY_BILATERAL_SPLAT_ID: &str = \
             \"rusttable.bilateral.splat\";"
        ));
        assert!(
            generated.contains("pub const ENTRY_EXPOSURE_ID: &str = \"rusttable.point.exposure\";")
        );
        assert!(
            registry.point_source().contains("fn exposure"),
            "point dispatch must keep the point program after other programs sort ahead of it"
        );
    }

    #[test]
    fn invalid_fixture_is_rejected_by_naga() {
        let source = include_str!("../../shaders/fixtures/invalid_syntax.wgsl");
        assert!(naga::front::wgsl::parse_str(source).is_err());
    }
}
