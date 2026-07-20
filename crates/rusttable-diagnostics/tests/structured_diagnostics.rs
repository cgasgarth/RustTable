use rusttable_diagnostics::{
    DiagnosticCode, DiagnosticEvent, DiagnosticField, PrivacyClass, Redactor, Severity, Subsystem,
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
