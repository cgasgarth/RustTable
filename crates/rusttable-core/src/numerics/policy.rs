use serde::{Deserialize, Serialize};

use super::NumericalContractError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FloatDomainPolicy {
    F32,
    F64Planning,
    F64Accumulation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NonFinitePolicy {
    Reject,
    Preserve,
    CanonicalizeNaN,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SubnormalPolicy {
    Preserve,
    FlushToZero,
    BackendDefined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FmaPolicy {
    SeparateRoundings,
    ExplicitFused,
    BackendDefined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReductionPolicy {
    None,
    SerialOrder,
    FixedTree { leaf_size: u16 },
    BackendDefined,
}

impl ReductionPolicy {
    /// Defines a fixed partition size and merge tree.
    ///
    /// # Errors
    ///
    /// Rejects zero or values that cannot be represented in the receipt schema.
    pub fn fixed_tree(leaf_size: usize) -> Result<Self, NumericalContractError> {
        u16::try_from(leaf_size)
            .ok()
            .filter(|value| *value != 0)
            .map(|leaf_size| Self::FixedTree { leaf_size })
            .ok_or(NumericalContractError::InvalidReductionLeafSize)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TranscendentalPolicy {
    None,
    RustIntrinsic,
    WgslBackend,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConversionRange {
    Reject,
    Clamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RoundingPolicy {
    NearestTiesEven,
    TowardZero,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConversionPolicy {
    pub range: ConversionRange,
    pub rounding: RoundingPolicy,
}

impl ConversionPolicy {
    #[must_use]
    pub const fn checked_nearest_even() -> Self {
        Self {
            range: ConversionRange::Reject,
            rounding: RoundingPolicy::NearestTiesEven,
        }
    }

    #[must_use]
    pub const fn clamped_nearest_even() -> Self {
        Self {
            range: ConversionRange::Clamp,
            rounding: RoundingPolicy::NearestTiesEven,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NumericalContract {
    pub float_domain: FloatDomainPolicy,
    pub non_finite: NonFinitePolicy,
    pub subnormal: SubnormalPolicy,
    pub fma: FmaPolicy,
    pub reduction: ReductionPolicy,
    pub transcendental: TranscendentalPolicy,
    pub conversion: ConversionPolicy,
}

impl NumericalContract {
    #[must_use]
    pub fn stable_id(self) -> String {
        format!(
            "{}-{}-{}-{}-{}-{}-{}",
            float_domain_id(self.float_domain),
            non_finite_id(self.non_finite),
            subnormal_id(self.subnormal),
            fma_id(self.fma),
            reduction_id(self.reduction),
            transcendental_id(self.transcendental),
            conversion_id(self.conversion),
        )
    }

    #[must_use]
    pub const fn has_backend_defined_behavior(self) -> bool {
        matches!(self.subnormal, SubnormalPolicy::BackendDefined)
            || matches!(self.fma, FmaPolicy::BackendDefined)
            || matches!(self.reduction, ReductionPolicy::BackendDefined)
            || matches!(self.transcendental, TranscendentalPolicy::WgslBackend)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToleranceClass {
    Exact,
    Transfer,
    Pointwise,
    Neighborhood,
    LegacyGpu,
}

impl ToleranceClass {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "Exact",
            Self::Transfer => "Transfer",
            Self::Pointwise => "Pointwise",
            Self::Neighborhood => "Neighborhood",
            Self::LegacyGpu => "LegacyGpu",
        }
    }

    #[must_use]
    pub const fn requires_matching_non_finite_locations(self) -> bool {
        true
    }
}

const fn float_domain_id(policy: FloatDomainPolicy) -> &'static str {
    match policy {
        FloatDomainPolicy::F32 => "f32",
        FloatDomainPolicy::F64Planning => "f64-planning",
        FloatDomainPolicy::F64Accumulation => "f64-accumulation",
    }
}

const fn non_finite_id(policy: NonFinitePolicy) -> &'static str {
    match policy {
        NonFinitePolicy::Reject => "reject",
        NonFinitePolicy::Preserve => "preserve-nonfinite",
        NonFinitePolicy::CanonicalizeNaN => "canonical-nan",
    }
}

const fn subnormal_id(policy: SubnormalPolicy) -> &'static str {
    match policy {
        SubnormalPolicy::Preserve => "preserve",
        SubnormalPolicy::FlushToZero => "ftz",
        SubnormalPolicy::BackendDefined => "backend-subnormal",
    }
}

const fn fma_id(policy: FmaPolicy) -> &'static str {
    match policy {
        FmaPolicy::SeparateRoundings => "separate",
        FmaPolicy::ExplicitFused => "fused",
        FmaPolicy::BackendDefined => "backend-fma",
    }
}

fn reduction_id(policy: ReductionPolicy) -> String {
    match policy {
        ReductionPolicy::None => "none".to_owned(),
        ReductionPolicy::SerialOrder => "serial".to_owned(),
        ReductionPolicy::FixedTree { leaf_size } => format!("fixed-{leaf_size}"),
        ReductionPolicy::BackendDefined => "backend-reduction".to_owned(),
    }
}

const fn transcendental_id(policy: TranscendentalPolicy) -> &'static str {
    match policy {
        TranscendentalPolicy::None => "none",
        TranscendentalPolicy::RustIntrinsic => "rust-intrinsic",
        TranscendentalPolicy::WgslBackend => "wgsl-backend",
    }
}

fn conversion_id(policy: ConversionPolicy) -> String {
    let range = match policy.range {
        ConversionRange::Reject => "checked",
        ConversionRange::Clamp => "clamped",
    };
    let rounding = match policy.rounding {
        RoundingPolicy::NearestTiesEven => "nearest-even",
        RoundingPolicy::TowardZero => "toward-zero",
    };
    format!("{range}-{rounding}")
}
