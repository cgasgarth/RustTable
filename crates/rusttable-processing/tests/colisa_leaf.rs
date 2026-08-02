#![forbid(unsafe_code)]
#![allow(
    clippy::assertions_on_constants,
    clippy::chunks_exact_to_as_chunks,
    clippy::float_cmp,
    reason = "source-derived metadata and fixture bytes require exact assertions"
)]

use std::cell::Cell;
use std::mem::size_of;

#[path = "../src/operations/colisa/mod.rs"]
mod colisa;

use colisa::{
    COLISA_METADATA, COLISA_PARAMETER_BYTES, COLISA_TABLE_ENTRIES, ColisaError, ColisaFormat,
    ColisaHistory, ColisaParametersV1, ColisaPlan, ColisaRaster, DEFAULT_V1_FIXTURE_HEX,
};

fn decode_hex(fixture: &str) -> Vec<u8> {
    let compact = fixture.trim();
    assert_eq!(compact.len() % 2, 0);
    compact
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            u8::from_str_radix(std::str::from_utf8(pair).expect("ASCII fixture"), 16)
                .expect("hex fixture")
        })
        .collect()
}

#[test]
fn native_fixture_and_opaque_history_round_trip() {
    let bytes = decode_hex(DEFAULT_V1_FIXTURE_HEX);
    assert_eq!(bytes.len(), COLISA_PARAMETER_BYTES);
    let history = ColisaHistory::decode(1, &bytes).expect("decode native v1");
    assert_eq!(
        history.current().expect("current v1"),
        ColisaParametersV1::defaults()
    );
    assert_eq!(history.payload().expect("encode v1"), bytes);
    assert_eq!(
        ColisaHistory::decode(1, &[0; 8]),
        Err(ColisaError::InvalidPayloadLength {
            expected: 12,
            actual: 8,
        })
    );

    let unknown_bytes = [9_u8, 8, 7, 6, 5, 4, 3];
    let unknown = ColisaHistory::decode(55, &unknown_bytes).expect("retain unknown history");
    assert_eq!(unknown.version(), 55);
    assert_eq!(
        unknown.payload().expect("copy unknown history"),
        unknown_bytes
    );
    assert_eq!(unknown.current(), Err(ColisaError::OpaqueVersion(55)));
}

#[test]
fn metadata_and_source_map_do_not_overclaim_routing() {
    assert_eq!(COLISA_METADATA.parameter_version, 1);
    assert!(COLISA_METADATA.deprecated);
    assert!(!COLISA_METADATA.default_enabled);
    assert_eq!(COLISA_METADATA.default_colorspace, "lab");
    assert!(COLISA_METADATA.allow_tiling);
    assert!(COLISA_METADATA.supports_shared_blending_native);
    assert!(!COLISA_METADATA.shared_blending_integrated);
    assert_eq!(COLISA_METADATA.legacy_order, 47.0);
    assert_eq!(COLISA_METADATA.v50_raw_order, 47.0);
    assert_eq!(COLISA_METADATA.v50_jpeg_order, 47.0);
    assert_eq!(COLISA_METADATA.generated_inventory_order, 74);

    let source_map = include_str!("../../../architecture/rusttable-colisa-cpu-source-map.toml");
    assert!(source_map.contains("production_registration = \"deferred"));
    assert!(source_map.contains("no fictitious v2 migration is invented"));
    for deferred in ["shared masks/blending", "GPU", "GTK", "history import"] {
        assert!(
            source_map.contains(deferred),
            "missing deferred responsibility: {deferred}"
        );
    }
}

#[test]
fn cpu_retains_lut_quantization_saturation_and_fourth_lane() {
    let table_bytes = 2 * COLISA_TABLE_ENTRIES * size_of::<f32>();
    let identity = ColisaPlan::compile(ColisaParametersV1::defaults(), table_bytes)
        .expect("compile identity plan");
    let input = [50.0_f32, 10.0, -20.0, 0.7];
    let output = identity
        .execute(
            ColisaRaster::new(&input, 1, 1, ColisaFormat::LabF32x4),
            input.len() * size_of::<f32>(),
            || false,
        )
        .expect("execute identity plan");
    assert_eq!(output[0].to_bits(), 50.0_f32.to_bits());
    assert_eq!(output[1].to_bits(), input[1].to_bits());
    assert_eq!(output[2].to_bits(), input[2].to_bits());
    assert_eq!(output[3].to_bits(), input[3].to_bits());

    let flat = ColisaPlan::compile(ColisaParametersV1::new(-1.0, 0.0, 0.5), table_bytes)
        .expect("compile flat contrast plan");
    let flat_output = flat
        .execute(
            ColisaRaster::new(&[12.0, 4.0, -6.0, 0.25], 1, 1, ColisaFormat::LabF32x4),
            4 * size_of::<f32>(),
            || false,
        )
        .expect("execute flat contrast plan");
    assert_eq!(flat_output[0].to_bits(), 50.0_f32.to_bits());
    assert_eq!(flat_output[1].to_bits(), 6.0_f32.to_bits());
    assert_eq!(flat_output[2].to_bits(), (-9.0_f32).to_bits());
    assert_eq!(flat_output[3].to_bits(), 0.25_f32.to_bits());
}

#[test]
fn planning_and_execution_fail_closed_transactionally() {
    let table_bytes = 2 * COLISA_TABLE_ENTRIES * size_of::<f32>();
    assert_eq!(
        ColisaPlan::compile(ColisaParametersV1::defaults(), table_bytes - 1),
        Err(ColisaError::WorkingMemoryBudgetExceeded {
            required: table_bytes,
            budget: table_bytes - 1,
        })
    );
    assert_eq!(
        ColisaPlan::compile(ColisaParametersV1::new(f32::NAN, 0.0, 0.0), table_bytes),
        Err(ColisaError::NonFiniteParameter("contrast"))
    );
    assert_eq!(
        ColisaPlan::compile(ColisaParametersV1::new(0.0, 1.01, 0.0), table_bytes),
        Err(ColisaError::ParameterOutOfRange("brightness"))
    );

    let plan = ColisaPlan::compile(ColisaParametersV1::defaults(), table_bytes)
        .expect("compile default plan");
    let valid = [50.0_f32, 1.0, 2.0, 0.5];
    assert_eq!(
        plan.execute(
            ColisaRaster::new(&valid, 1, 1, ColisaFormat::RgbaF32x4),
            usize::MAX,
            || false,
        ),
        Err(ColisaError::UnsupportedFormat)
    );
    assert_eq!(
        plan.execute(
            ColisaRaster::new(&valid[..3], 1, 1, ColisaFormat::LabF32x4),
            usize::MAX,
            || false,
        ),
        Err(ColisaError::InputLengthMismatch {
            expected: 4,
            actual: 3,
        })
    );
    assert_eq!(
        plan.execute(
            ColisaRaster::new(&valid, 1, 1, ColisaFormat::LabF32x4),
            15,
            || false,
        ),
        Err(ColisaError::OutputMemoryBudgetExceeded {
            required: 16,
            budget: 15,
        })
    );
    let invalid = [50.0_f32, 1.0, f32::NEG_INFINITY, 0.5];
    assert_eq!(
        plan.execute(
            ColisaRaster::new(&invalid, 1, 1, ColisaFormat::LabF32x4),
            usize::MAX,
            || false,
        ),
        Err(ColisaError::NonFiniteInput { index: 2 })
    );

    let many = vec![50.0_f32; 257 * 4];
    let polls = Cell::new(0_u32);
    let mut destination = vec![3.0_f32, 2.0, 1.0];
    let error = plan
        .execute_and_publish(
            ColisaRaster::new(&many, 257, 1, ColisaFormat::LabF32x4),
            &mut destination,
            usize::MAX,
            || {
                let next = polls.get() + 1;
                polls.set(next);
                next >= 3
            },
        )
        .expect_err("cancel before publication");
    assert_eq!(error, ColisaError::Cancelled);
    assert_eq!(destination, [3.0_f32, 2.0, 1.0]);
}
