//! Source-derived bounded Monochrome CPU leaf coverage.
//!
//! The operation is included by path: registry, production history, pixelpipe,
//! GPU, GTK/profile-panel, outer blending, and preset integration are separate
//! deferred seams.

#![allow(
    clippy::assertions_on_constants,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    clippy::needless_range_loop,
    clippy::unreadable_literal,
    clippy::too_many_lines,
    reason = "source-derived vectors assert native f32 bytes and fixed field order"
)]

#[path = "../src/operations/monochrome/mod.rs"]
mod monochrome;

use monochrome::source_map::{MONOCHROME_SOURCE_MAP, MonochromePortStatus};
use monochrome::{
    MONOCHROME_ALLOW_TILING, MONOCHROME_DEFAULT_COLORSPACE, MONOCHROME_DEFAULT_GROUPS,
    MONOCHROME_GPU_KERNELS, MONOCHROME_GPU_PROGRAM, MONOCHROME_MIGRATION_EDGES,
    MONOCHROME_SCHEMA_VERSION, MONOCHROME_SUPPORTS_BLENDING, MONOCHROME_V1_PARAMETER_BYTES,
    MONOCHROME_V2_PARAMETER_BYTES, MonochromeCodecError, MonochromeConfig,
    MonochromeExecutionError, MonochromeHistory, MonochromeParametersV1, MonochromeParametersV2,
    MonochromePixel, MonochromePlan, capabilities, envelope, fast_expf,
};

const DEFAULT_FIXTURE: &str = include_str!("fixtures/monochrome/default-v2.hex");

#[test]
fn native_v2_abi_defaults_and_lineage_fixture_are_exact() {
    assert_eq!(MONOCHROME_SCHEMA_VERSION, 2);
    assert_eq!(MONOCHROME_V1_PARAMETER_BYTES, 12);
    assert_eq!(MONOCHROME_V2_PARAMETER_BYTES, 16);
    assert_eq!(MONOCHROME_MIGRATION_EDGES, &[(1, 2)]);
    assert_eq!(MONOCHROME_DEFAULT_GROUPS, ["color", "effects"]);
    assert_eq!(MONOCHROME_DEFAULT_COLORSPACE, "Lab");
    assert!(MONOCHROME_SUPPORTS_BLENDING);
    assert!(MONOCHROME_ALLOW_TILING);
    assert_eq!(MONOCHROME_GPU_PROGRAM, 2);
    assert_eq!(MONOCHROME_GPU_KERNELS, ["monochrome_filter", "monochrome"]);

    let defaults = MonochromeParametersV2::defaults();
    assert_eq!(defaults.a, 0.0);
    assert_eq!(defaults.b, 0.0);
    assert_eq!(defaults.size, 2.0);
    assert_eq!(defaults.highlights, 0.0);
    assert_eq!(
        defaults.to_bytes(),
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 64, 0, 0, 0, 0]
    );
    assert!(DEFAULT_FIXTURE.contains("payload_bytes=16"));
    assert!(DEFAULT_FIXTURE.contains("payload_hex=00000000000000000000004000000000"));
    assert!(DEFAULT_FIXTURE.contains("red_filter_fields=[a:32.0,b:64.0,size:2.3,highlights:0.0]"));
    assert!(DEFAULT_FIXTURE.contains(
        "cpu_output_bits=[[1103310910,0,0,1040187392],[1111944171,0,0,1048576000],[1116869771,0,0,1056964608],[1120088126,0,0,1063256064]]"
    ));
}

#[test]
fn native_v1_migration_preserves_fields_and_adds_zero_highlights() {
    let v1 = MonochromeParametersV1::new(-32.0, 64.0, 2.3);
    let history = MonochromeHistory::decode(1, &v1.to_bytes()).expect("valid v1 history");
    assert_eq!(history.version(), 1);
    assert_eq!(history.payload(), v1.to_bytes());
    assert_eq!(
        history.current().expect("v1 migrates"),
        MonochromeParametersV2::new(-32.0, 64.0, 2.3, 0.0)
    );
    assert!(DEFAULT_FIXTURE.contains("migration=(1,2)"));

    let current = MonochromeHistory::decode(2, &MonochromeParametersV2::defaults().to_bytes())
        .expect("valid v2 history");
    assert_eq!(current.current().expect("v2 is current"), defaults());
}

#[test]
fn malformed_known_payloads_fail_and_future_payloads_round_trip_opaque() {
    assert_eq!(
        MonochromeHistory::decode(1, &[0; 11]),
        Err(MonochromeCodecError::InvalidLength {
            expected: 12,
            actual: 11,
        })
    );
    assert_eq!(
        MonochromeHistory::decode(2, &[0; 15]),
        Err(MonochromeCodecError::InvalidLength {
            expected: 16,
            actual: 15,
        })
    );

    let future = vec![0xde, 0xad, 0xbe, 0xef, 0x01];
    let history = MonochromeHistory::decode(99, &future).expect("future remains opaque");
    assert_eq!(history.version(), 99);
    assert_eq!(history.payload(), future);
    assert_eq!(
        history.current(),
        Err(MonochromeCodecError::UnsupportedVersion(99))
    );
}

#[test]
fn finite_commit_state_keeps_native_ui_ranges_out_of_execution_validation() {
    assert!(MonochromeConfig::new(0.0, 0.0, 2.0, -12.0).is_ok());
    assert!(matches!(
        MonochromeConfig::new(f32::NAN, 0.0, 2.0, 0.0),
        Err(monochrome::MonochromeParameterError::NonFinite("a"))
    ));
    assert!(matches!(
        MonochromeConfig::new(0.0, 0.0, f32::INFINITY, 0.0),
        Err(monochrome::MonochromeParameterError::NonFinite("size"))
    ));
}

#[test]
fn fast_exp_and_envelope_keep_native_boundaries() {
    assert_eq!(fast_expf(0.0).to_bits(), 1.0_f32.to_bits());
    assert!(fast_expf(-1.0).is_finite());
    assert_eq!(envelope(-20.0), 0.0);
    assert_eq!(envelope(0.0), 0.0);
    assert_eq!(envelope(60.0), 1.0);
    assert_eq!(envelope(100.0), 0.0);
    assert_eq!(envelope(120.0), 0.0);
}

#[test]
fn cpu_execution_zeroes_lab_chroma_preserves_alpha_and_publishes_only_complete_output() {
    let config = MonochromeConfig::defaults();
    let dimensions = rusttable_processing::RasterDimensions::new(2, 2).expect("dimensions");
    let plan = MonochromePlan::new(config, dimensions).expect("plan");
    let input = vec![
        MonochromePixel::new(25.0, -32.0, 64.0, 0.125),
        MonochromePixel::new(50.0, 0.0, 0.0, 0.25),
        MonochromePixel::new(75.0, 32.0, -64.0, 0.5),
        MonochromePixel::new(100.0, 64.0, 32.0, 0.875),
    ];
    let output = plan.execute(&input).expect("CPU output");
    assert_eq!(output.len(), input.len());
    assert_eq!(
        output
            .iter()
            .map(|pixel| pixel.channels().map(f32::to_bits))
            .collect::<Vec<_>>(),
        vec![
            [1103310910, 0, 0, 1040187392],
            [1111944171, 0, 0, 1048576000],
            [1116869771, 0, 0, 1056964608],
            [1120088126, 0, 0, 1063256064],
        ]
    );
    for (source, result) in input.into_iter().zip(output) {
        assert_eq!(result.a().to_bits(), 0.0_f32.to_bits());
        assert_eq!(result.b().to_bits(), 0.0_f32.to_bits());
        assert_eq!(result.alpha().to_bits(), source.alpha().to_bits());
        assert!(result.lightness().is_finite());
    }

    let mut calls = 0;
    let cancelled = plan.execute_with_cancel(
        &[
            MonochromePixel::new(25.0, 0.0, 0.0, 0.1),
            MonochromePixel::new(50.0, 0.0, 0.0, 0.2),
            MonochromePixel::new(75.0, 0.0, 0.0, 0.3),
            MonochromePixel::new(100.0, 0.0, 0.0, 0.4),
        ],
        || {
            calls += 1;
            calls > 2
        },
    );
    assert_eq!(cancelled, Err(MonochromeExecutionError::Cancelled));
}

#[test]
fn bilateral_tiling_and_capabilities_match_bounded_contract() {
    let dimensions = rusttable_processing::RasterDimensions::new(8, 6).expect("dimensions");
    let plan = MonochromePlan::new(MonochromeConfig::defaults(), dimensions).expect("plan");
    assert_eq!(
        plan.config().parameters(),
        MonochromeParametersV2::defaults()
    );
    assert_eq!(plan.dimensions(), dimensions);
    assert_eq!(plan.sigma_s(), 20.0);
    let tiling = plan.tiling(4, 1).expect("tiling");
    assert_eq!(tiling.overhead, 0);
    assert_eq!(tiling.overlap, 80);
    assert_eq!(tiling.align, 1);
    assert!(tiling.factor > 2.0);
    assert!(tiling.factor_cl > tiling.factor);
    assert_eq!(tiling.maxbuf, tiling.maxbuf_cl);

    let scaled = MonochromePlan::new_with_scale(MonochromeConfig::defaults(), dimensions, 0.5, 1.0)
        .expect("scaled plan");
    assert_eq!(scaled.sigma_s(), 20.0);
    assert_eq!(scaled.tiling(4, 1).expect("scaled tiling").overlap, 160);

    let support = capabilities();
    assert!(support.cpu_supported);
    assert!(!support.gpu_supported);
    assert!(!support.gtk_supported);
    assert!(!support.masks_consumed);
    assert!(support.outer_blending_deferred);
    assert!(support.production_routing_deferred);
    assert!(support.alpha_preserved);
    assert!(support.require_gpu().is_err());
    assert!(support.require_gtk().is_err());
    assert!(support.require_production_routing().is_err());
}

#[test]
fn source_map_covers_ported_and_explicitly_deferred_surfaces() {
    assert!(MONOCHROME_SOURCE_MAP.iter().any(|entry| {
        entry.native_symbol.contains("legacy_params")
            && entry.status == MonochromePortStatus::Ported
    }));
    assert!(MONOCHROME_SOURCE_MAP.iter().any(|entry| {
        entry.native_file == "data/kernels/monochrome.cl"
            && entry.status == MonochromePortStatus::ExplicitlyDeferred
    }));
    assert!(MONOCHROME_SOURCE_MAP.iter().any(|entry| {
        entry.native_symbol.contains("gui_init")
            && entry.status == MonochromePortStatus::ExplicitlyDeferred
    }));
}

fn defaults() -> MonochromeParametersV2 {
    MonochromeParametersV2::defaults()
}
