use rusttable_diagnostics::{
    DiagnosticCode, DiagnosticEvent, DiagnosticField, PrivacyClass, Redactor,
    SelectedPreviewFailureCode, SelectedPreviewFailureStage, SelectedPreviewMetadata,
    SelectedPreviewOperation, Severity, Subsystem,
};

#[test]
fn codes_and_fields_are_checked_and_classified() {
    let subsystem = Subsystem::new("import").expect("valid subsystem");
    let code = DiagnosticCode::new(subsystem, "source.open").expect("valid code");
    assert_eq!(code.as_str(), "import.source.open");
    assert!(Subsystem::new("Import").is_err());
    assert!(DiagnosticCode::new(code.subsystem().clone(), "not valid!").is_err());

    assert_eq!(
        DiagnosticField::path("/private/photo.raw")
            .unwrap()
            .privacy(),
        PrivacyClass::Private
    );
    assert_eq!(
        DiagnosticField::credential("secret").unwrap().privacy(),
        PrivacyClass::Secret
    );
    assert_eq!(
        DiagnosticField::pixel_data(&[1, 2, 3]).unwrap().privacy(),
        PrivacyClass::Payload
    );
    assert!(DiagnosticField::public_text("field", &"x".repeat(4 * 1024 + 1)).is_err());

    let event = DiagnosticEvent::new(code, Severity::Warning, "open")
        .expect("valid event")
        .with_field(DiagnosticField::public_text("format", "raw").unwrap())
        .expect("field fits");
    assert_eq!(event.severity(), Severity::Warning);
    assert_eq!(event.operation(), "open");
}

#[test]
fn aliases_are_stable_only_within_one_process() {
    let first = Redactor::new();
    let second = Redactor::new();
    assert_eq!(
        first.alias("/private/photo.raw"),
        first.alias("/private/photo.raw")
    );
    assert_ne!(
        first.alias("/private/photo.raw"),
        second.alias("/private/photo.raw")
    );
    assert!(
        !format!("{:?}", DiagnosticField::credential("do-not-print").unwrap())
            .contains("do-not-print")
    );
}

#[test]
fn selected_preview_taxonomy_covers_all_stages_and_uses_typed_causes() {
    let stages = [
        SelectedPreviewFailureStage::CatalogLookup,
        SelectedPreviewFailureStage::EditSelection,
        SelectedPreviewFailureStage::SourceDecode,
        SelectedPreviewFailureStage::Processing,
        SelectedPreviewFailureStage::HistogramGeneration,
        SelectedPreviewFailureStage::TextureAdaptation,
        SelectedPreviewFailureStage::ImportPreview,
        SelectedPreviewFailureStage::StaleResult,
    ];
    let names = stages.map(SelectedPreviewFailureStage::as_str);
    assert_eq!(
        names,
        [
            "catalog_lookup",
            "edit_selection",
            "source_decode",
            "processing",
            "histogram_generation",
            "texture_adaptation",
            "import_preview",
            "stale_result",
        ]
    );

    let event = DiagnosticEvent::selected_preview_failure(
        SelectedPreviewFailureStage::SourceDecode,
        SelectedPreviewFailureCode::UnsupportedFormat,
        SelectedPreviewOperation::DecodeSource,
    )
    .with_selected_preview_metadata(
        SelectedPreviewMetadata::default()
            .with_generation(12)
            .with_expected_generation(13)
            .with_dimensions(1920, 1080)
            .expect("valid dimensions")
            .with_byte_length(8_294_400)
            .with_format("raw")
            .expect("valid format")
            .with_source_kind("raw")
            .expect("valid source kind"),
    )
    .expect("metadata fits");
    assert_eq!(event.code().as_str(), "preview.selected_failure");
    assert_eq!(event.operation(), "decode_source");
}

#[test]
fn selected_preview_metadata_updates_in_place_and_stays_bounded() {
    let metadata = SelectedPreviewMetadata::default()
        .with_generation(1)
        .with_generation(2)
        .with_expected_generation(3)
        .with_dimensions(1, 2)
        .expect("valid dimensions")
        .with_byte_length(8)
        .with_format("rgba8")
        .expect("valid format")
        .with_source_kind("raster")
        .expect("valid source kind");
    let event = DiagnosticEvent::selected_preview_failure(
        SelectedPreviewFailureStage::StaleResult,
        SelectedPreviewFailureCode::StaleGeneration,
        SelectedPreviewOperation::DiscardStaleResult,
    )
    .with_selected_preview_metadata(metadata)
    .expect("metadata fits");
    assert_eq!(event.operation(), "discard_stale_result");
}
