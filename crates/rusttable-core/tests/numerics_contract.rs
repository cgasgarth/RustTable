use rusttable_core::numerics::{
    CompilerBaseline, ConversionPolicy, FloatDomainPolicy, FmaPolicy, ImplementationFamily,
    ImplementationNumerics, NonFinitePolicy, NumericalContract, ReductionPolicy, SubnormalPolicy,
    ToleranceClass, TranscendentalPolicy,
    conversion::{canonical_f16_bits, f32_to_u8},
    inspection::ulp_distance_f32,
    ordered::{
        finite_divide_f32, normalize_circular_f32, ordered_clamp_f32, ordered_max_f32,
        ordered_min_f32,
    },
    reduction::{
        BivariateMoments, ReductionPlan, deterministic_sum_f32, deterministic_sum_f64,
        merge_histograms, merge_indexed,
    },
};

fn strict_contract() -> NumericalContract {
    NumericalContract {
        float_domain: FloatDomainPolicy::F32,
        non_finite: NonFinitePolicy::Reject,
        subnormal: SubnormalPolicy::Preserve,
        fma: FmaPolicy::SeparateRoundings,
        reduction: ReductionPolicy::fixed_tree(3).expect("leaf size"),
        transcendental: TranscendentalPolicy::None,
        conversion: ConversionPolicy::checked_nearest_even(),
    }
}

#[test]
fn implementation_metadata_rejects_unregistered_relaxations() {
    let metadata = ImplementationNumerics::new(
        "rusttable.test.scalar",
        "rusttable.test.reference",
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        ImplementationFamily::Scalar,
        CompilerBaseline::PrimaryBeta,
        ToleranceClass::Exact,
        strict_contract(),
    )
    .expect("complete metadata");
    assert_eq!(
        metadata.contract().stable_id(),
        "f32-reject-preserve-separate-fixed-3-none-checked-nearest-even"
    );

    let mut relaxed = strict_contract();
    relaxed.fma = FmaPolicy::BackendDefined;
    assert!(
        ImplementationNumerics::new(
            "rusttable.test.gpu",
            "rusttable.test.reference",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            ImplementationFamily::Gpu,
            CompilerBaseline::BackendToolchain,
            ToleranceClass::Exact,
            relaxed,
        )
        .is_err(),
        "backend-defined rounding cannot claim exact parity"
    );
}

#[test]
fn conversions_and_ordered_helpers_cover_adversarial_boundaries() {
    let policy = ConversionPolicy::clamped_nearest_even();
    assert_eq!(f32_to_u8(254.5, policy).expect("tie"), 254);
    assert_eq!(f32_to_u8(255.5, policy).expect("clamp"), 255);
    assert_eq!(f32_to_u8(-0.5, policy).expect("clamp"), 0);
    assert!(f32_to_u8(f32::NAN, policy).is_err());

    assert_eq!(
        canonical_f16_bits(-0.0, NonFinitePolicy::Reject).unwrap(),
        0x8000
    );
    assert!(canonical_f16_bits(f32::INFINITY, NonFinitePolicy::Reject).is_err());
    assert_eq!(
        ordered_clamp_f32(-0.0, 0.0, 1.0, NonFinitePolicy::Reject)
            .unwrap()
            .to_bits(),
        0.0_f32.to_bits()
    );
    assert!(ordered_clamp_f32(0.0, 2.0, 1.0, NonFinitePolicy::Reject).is_err());
    assert_eq!(
        ordered_min_f32(-0.0, 0.0, NonFinitePolicy::Reject)
            .unwrap()
            .to_bits(),
        (-0.0_f32).to_bits()
    );
    assert_eq!(
        ordered_max_f32(-0.0, 0.0, NonFinitePolicy::Reject)
            .unwrap()
            .to_bits(),
        0.0_f32.to_bits()
    );
    let payload_nan = f32::from_bits(0x7fc0_0042);
    assert_eq!(
        ordered_min_f32(1.0, payload_nan, NonFinitePolicy::Preserve)
            .unwrap()
            .to_bits(),
        payload_nan.to_bits()
    );
    assert!(finite_divide_f32(1.0, 0.0).is_err());
    assert_eq!(
        normalize_circular_f32(-1.0, 360.0).unwrap().to_bits(),
        359.0_f32.to_bits()
    );
    assert_eq!(
        ulp_distance_f32(1.0, f32::from_bits(1.0_f32.to_bits() + 1)).unwrap(),
        1
    );
    assert!(ulp_distance_f32(f32::NAN, 1.0).is_err());
}

#[test]
fn fixed_reduction_tree_is_independent_of_completed_leaf_order() {
    let values = [1.0e20_f32, 1.0, -1.0e20, 3.0, f32::MIN_POSITIVE, -0.0, 7.0];
    let plan = ReductionPlan::new(values.len(), 3).expect("plan");
    let expected = deterministic_sum_f32(&values, &plan).expect("sum");
    let mut partials = plan
        .ranges()
        .iter()
        .enumerate()
        .map(|(index, range)| {
            let value = values[range.clone()]
                .iter()
                .copied()
                .fold(0.0, |a, b| a + b);
            (index, value)
        })
        .collect::<Vec<_>>();
    for shift in 0..partials.len() {
        partials.rotate_left(shift);
        if shift.is_multiple_of(2) {
            partials.reverse();
        }
        let merged = merge_indexed(
            plan.leaf_count(),
            partials.clone(),
            0.0_f32,
            |left, right| left + right,
        )
        .expect("merge");
        assert_eq!(expected.to_bits(), merged.to_bits());
    }

    let duplicate = vec![(0, 1.0_f32), (0, 2.0)];
    assert!(merge_indexed(2, duplicate, 0.0, |left, right| left + right).is_err());
}

#[test]
fn sums_and_histograms_share_fixed_partition_identity() {
    let values = [f64::MAX / 4.0, -f64::MAX / 4.0, 0.5, 0.25, -0.0];
    let plan = ReductionPlan::new(values.len(), 2).expect("plan");
    assert!(deterministic_sum_f64(&values, &plan).unwrap().is_finite());

    let mut leaves = vec![(0, vec![1, 2, 3]), (1, vec![4, 5, 6]), (2, vec![7, 8, 9])];
    let expected = merge_histograms(3, 3, leaves.clone()).expect("histogram");
    assert_eq!(expected, vec![12, 15, 18]);
    leaves.rotate_left(1);
    assert_eq!(merge_histograms(3, 3, leaves).unwrap(), expected);
    assert!(merge_histograms(2, 2, [(0, vec![u64::MAX, 0]), (1, vec![1, 0])]).is_err());
}

#[test]
fn deterministic_bivariate_moments_use_the_same_tree_for_covariance_inputs() {
    let samples = [
        (1.0, 2.0),
        (2.0, 4.0),
        (3.0, 8.0),
        (-0.0, f64::MIN_POSITIVE),
    ];
    let plan = ReductionPlan::new(samples.len(), 2).expect("plan");
    let first = BivariateMoments::reduce(&samples, &plan).expect("moments");
    let second = BivariateMoments::reduce(&samples, &plan).expect("moments");
    assert_eq!(first, second);
    assert_eq!(first.count(), 4);
    assert!(
        first
            .covariance_population()
            .expect("covariance")
            .is_finite()
    );
}
