#![allow(clippy::cast_precision_loss)]

use rusttable_masks::MaskRaster;
use rusttable_processing::{
    LinearRgb, RasterDimensions, SpotsConfig, SpotsEntry, SpotsForm, SpotsFormKind, SpotsHistory,
    SpotsLegacySpot, SpotsMode, SpotsParametersV1, SpotsParametersV2, SpotsPlan,
    descriptor::OperationFlags,
};

fn dimensions() -> RasterDimensions {
    RasterDimensions::new(4, 1).expect("dimensions")
}

fn pixel(value: f32) -> LinearRgb {
    LinearRgb::new(
        rusttable_processing::FiniteF32::new(value).expect("finite"),
        rusttable_processing::FiniteF32::new(value).expect("finite"),
        rusttable_processing::FiniteF32::new(value).expect("finite"),
    )
}

fn mask(index: usize) -> MaskRaster {
    MaskRaster::new(
        4,
        1,
        (0..4)
            .map(|value| f32::from(u8::from(value == index)))
            .collect(),
    )
    .expect("mask")
}

fn form(id: u32, index: usize) -> SpotsForm {
    SpotsForm::from_mask(id, mask(index), SpotsFormKind::Circle).expect("form")
}

#[test]
fn codecs_preserve_v1_padding_and_v2_ordered_modes() {
    let legacy = SpotsParametersV1::new(vec![
        SpotsLegacySpot::new(1.0, 0.5, 0.0, 0.5, 2.0).expect("spot"),
    ])
    .expect("v1");
    let mut bytes = legacy.to_bytes();
    bytes[700] = 0x5a;
    let decoded = SpotsParametersV1::from_bytes(&bytes).expect("decode v1");
    assert_eq!(decoded.spots().len(), 1);
    assert_eq!(decoded.padding()[56], 0x5a);
    assert_eq!(decoded.to_bytes(), bytes);

    let current = SpotsParametersV2::from_entries([(42, SpotsMode::Clone), (7, SpotsMode::Heal)])
        .expect("v2");
    assert_eq!(
        SpotsParametersV2::from_bytes(&current.to_bytes()).expect("roundtrip"),
        current
    );
    assert_eq!(
        current.ordered_entries(),
        vec![(42, SpotsMode::Clone), (7, SpotsMode::Heal)]
    );
    assert!(
        !SpotsHistory::decode(99, &[1, 2, 3])
            .expect("opaque")
            .executable()
    );
}

#[test]
fn v1_migration_assigns_stable_form_ids_and_heal_mode() {
    let legacy = SpotsParametersV1::new(vec![
        SpotsLegacySpot::new(0.0, 0.0, 1.0, 0.0, 1.0).expect("spot"),
        SpotsLegacySpot::new(2.0, 0.0, 3.0, 0.0, 1.0).expect("spot"),
    ])
    .expect("v1");
    let migrated =
        rusttable_processing::operations::spots::migrate_v1_to_v2(&legacy).expect("migration");
    assert_eq!(
        migrated.ordered_entries(),
        vec![(1, SpotsMode::Heal), (2, SpotsMode::Heal)]
    );
}

#[test]
fn ordered_spots_read_immutable_input_and_union_source_destination_rois() {
    let config = SpotsConfig::new()
        .with_form(form(1, 1))
        .with_form(form(2, 2))
        .with_entry(SpotsEntry::new(1, SpotsMode::Clone, (-1, 0), 1.0).expect("entry"))
        .with_entry(SpotsEntry::new(2, SpotsMode::Clone, (-1, 0), 1.0).expect("entry"));
    let plan = SpotsPlan::new(config, dimensions()).expect("plan");
    assert_eq!(plan.destination_roi().x(), 1);
    assert_eq!(plan.destination_roi().width(), 2);
    assert_eq!(plan.source_roi().x(), 0);
    assert_eq!(plan.source_roi().width(), 2);
    let input = [pixel(0.0), pixel(10.0), pixel(20.0), pixel(30.0)];
    let (output, receipt) = plan.execute(&input, || false).expect("execute");
    assert_eq!(output[1], pixel(0.0));
    assert_eq!(output[2], pixel(10.0));
    assert_eq!(receipt.spot_count(), 2);
}

#[test]
fn heal_is_distinct_from_clone_and_cancellation_publishes_no_partial_output() {
    let heal = SpotsConfig::new()
        .with_form(form(1, 1))
        .with_entry(SpotsEntry::new(1, SpotsMode::Heal, (-1, 0), 1.0).expect("entry"));
    let plan = SpotsPlan::new(heal, dimensions()).expect("plan");
    let input = [pixel(1.0), pixel(2.0), pixel(3.0), pixel(4.0)];
    let (output, _) = plan.execute(&input, || false).expect("heal");
    assert_eq!(output[1], pixel(2.0));

    let calls = std::cell::Cell::new(0);
    let cancelled = plan.execute(&input, || {
        calls.set(calls.get() + 1);
        calls.get() > 1
    });
    assert!(cancelled.is_err());
    assert_eq!(input[1], pixel(2.0));
}

#[test]
fn registry_marks_spots_deprecated_hidden_and_cpu_deterministic() {
    let definition = rusttable_processing::builtin_registry()
        .definition("rusttable.spots")
        .expect("spots registry entry");
    assert!(
        definition
            .descriptor()
            .flags
            .contains(OperationFlags::DEPRECATED)
    );
    assert!(
        definition
            .descriptor()
            .flags
            .contains(OperationFlags::HIDDEN)
    );
    assert!(
        definition
            .descriptor()
            .flags
            .contains(OperationFlags::DETERMINISTIC_CPU)
    );
    assert_eq!(
        definition.descriptor().migration.source_versions,
        vec![1, 2]
    );
    assert!(definition.cpu().is_some());
}

#[test]
fn source_outside_image_is_rejected_before_execution() {
    let config = SpotsConfig::new()
        .with_form(form(1, 0))
        .with_entry(SpotsEntry::new(1, SpotsMode::Clone, (-1, 0), 1.0).expect("entry"));
    assert!(matches!(
        SpotsPlan::new(config, dimensions()),
        Err(rusttable_processing::SpotsExecutionError::SourceOutOfBounds { form_id: 1 })
    ));
}
