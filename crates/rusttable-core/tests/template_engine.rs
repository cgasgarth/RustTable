use std::panic::{AssertUnwindSafe, catch_unwind};

use rusttable_core::template::{
    BuiltinTemplate, Dimensions, EncoderDescriptor, EvaluationError, Rational, SanitizerPolicy,
    Template, TemplateContext, TemplateDateTime, TemplateValue, VariableId, VariableValue,
    sanitize_component,
};

fn context() -> TemplateContext {
    let mut context = TemplateContext::new();
    context.set_text(VariableId::SourceStem, "Café/holiday");
    context.set_text(VariableId::Title, "Private title");
    context.set_integer(VariableId::Sequence, 7);
    context.set_integer(VariableId::VirtualCopy, 2);
    context.set(
        VariableId::CaptureDate,
        VariableValue::available(TemplateValue::DateTime(
            TemplateDateTime::new(2026, 7, 18, 14, 5, 6, -240).expect("date"),
        )),
    );
    context.set(
        VariableId::Aperture,
        VariableValue::available(TemplateValue::Rational(
            Rational::new(28, 10).expect("rational"),
        )),
    );
    context.set(
        VariableId::Width,
        VariableValue::available(TemplateValue::Integer(4_000)),
    );
    context.set(
        VariableId::Height,
        VariableValue::available(TemplateValue::Integer(3_000)),
    );
    context.set_tags(vec![
        "zebra".to_owned(),
        "café".to_owned(),
        "alpha".to_owned(),
    ]);
    context
}

#[test]
fn template_engine_parses_escapes_fallback_conditionals_and_transforms() {
    let template =
        Template::parse("exports/$$/${?title:${slug:${title}}:untitled}").expect("template");
    let (name, receipt) = template
        .evaluate(
            &context(),
            Some(&EncoderDescriptor::new("jpeg", "jpg", &["jpg", "jpeg"])),
        )
        .expect("evaluation");
    assert_eq!(name.relative_path, "exports/$/private-title");
    assert_eq!(name.components.len(), 3);
    assert_eq!(receipt.schema_version, 1);
    assert_eq!(receipt.receipt_hash().len(), 64);
}

#[test]
fn template_engine_formats_dates_photographic_values_and_tags_deterministically() {
    let mut context = context();
    context.set_text(VariableId::Extension, "jpg");
    let template = Template::parse(
        "${capture_date:%Y-%m-%d}/${capture_date:%H%M%S}-${aperture:.1}-${tags:join=_}-${width}x${height}.${extension}",
    )
    .expect("template");
    let encoder = EncoderDescriptor::new("jpeg", "jpg", &["jpg", "jpeg"]);
    let (first, _) = template
        .evaluate(&context, Some(&encoder))
        .expect("evaluation");
    let (second, _) = template
        .evaluate(&context, Some(&encoder))
        .expect("evaluation");
    assert_eq!(first, second);
    assert_eq!(
        first.relative_path,
        "2026-07-18/140506-2.8-alpha_café_zebra-4000x3000.jpg"
    );
}

#[test]
fn template_engine_distinguishes_missing_and_redacted_values() {
    let fallback = Template::parse("${title|untitled}").expect("template");
    let (name, _) = fallback
        .evaluate(&TemplateContext::new(), None)
        .expect("fallback");
    assert_eq!(name.relative_path, "untitled");

    let mut redacted_context = TemplateContext::new();
    redacted_context.set(VariableId::Title, VariableValue::redacted());
    let conditional = Template::parse("${?title:private:public}").expect("conditional");
    assert!(matches!(
        conditional.evaluate(&redacted_context, None),
        Err(EvaluationError::RedactedVariable {
            id: VariableId::Title
        })
    ));

    let actual = Template::parse("${title}").expect("template");
    let (name, receipt) = actual.evaluate(&context(), None).expect("actual");
    assert_eq!(name.relative_path, "Private title");
    assert_eq!(receipt.display_path, "[redacted]");
    assert!(receipt.privacy_redacted);
}

#[test]
fn template_engine_rejects_invalid_boundaries_and_extensions() {
    assert!(Template::parse("/absolute").is_err());
    assert!(Template::parse("C:/drive").is_err());
    assert!(Template::parse("one/../two").is_err());
    assert!(Template::parse("${unknown}").is_err());
    assert!(Template::parse("${extension}/nested").is_err());
    assert!(
        Template::parse(
            "${sequence:00000000000000000000000000000000000000000000000000000000000000000}"
        )
        .is_err()
    );

    let template = Template::parse("${source_stem}.${extension}").expect("template");
    let mut context = TemplateContext::new();
    context.set_text(VariableId::SourceStem, "photo");
    context.set_text(VariableId::Extension, "png");
    let encoder = EncoderDescriptor::new("jpeg", "jpg", &["jpg", "jpeg"]);
    assert!(matches!(
        template.evaluate(&context, Some(&encoder)),
        Err(EvaluationError::InvalidExtension { .. })
    ));
}

#[test]
fn template_engine_sanitizer_is_portable_and_does_not_split_utf8() {
    let reserved = sanitize_component("CON.txt. ", SanitizerPolicy::default()).expect("sanitized");
    assert_eq!(reserved.value, "_CON.txt");
    let decomposed =
        sanitize_component("cafe\u{301}", SanitizerPolicy::default()).expect("sanitized");
    assert_eq!(decomposed.value, "café");
    let long = sanitize_component(
        &"🦀".repeat(200),
        SanitizerPolicy {
            max_component_bytes: 31,
            ..SanitizerPolicy::default()
        },
    )
    .expect("truncated");
    assert!(long.value.len() <= 31);
    assert!(long.value.is_char_boundary(long.value.len()));
}

#[test]
fn template_engine_builtin_hashes_and_bounded_fuzz_inputs_are_stable() {
    let mut first_hashes = Vec::new();
    for builtin in BuiltinTemplate::all() {
        let template = builtin.template().expect("builtin");
        first_hashes.push((builtin.name(), builtin.content_hash(), template.ast_hash()));
    }
    for builtin in BuiltinTemplate::all() {
        let template = builtin.template().expect("builtin");
        assert_eq!(builtin.content_hash(), template.ast_hash());
    }
    assert_eq!(first_hashes.len(), 5);

    for seed in 0_u32..512 {
        let source = format!(
            "x{}${{sequence:02}}{}🦀",
            seed,
            char::from_u32(seed % 128).expect("ascii")
        );
        let result = catch_unwind(AssertUnwindSafe(|| Template::parse(&source)));
        assert!(result.is_ok(), "parser panicked for seed {seed}");
        let malformed = format!("${{source_stem:{seed}");
        assert!(catch_unwind(AssertUnwindSafe(|| Template::parse(&malformed))).is_ok());
    }
}

#[test]
fn template_engine_accepts_typed_dimension_values_for_aspect_formatting() {
    let mut context = TemplateContext::new();
    context.set(
        VariableId::Aspect,
        VariableValue::available(TemplateValue::Dimensions(
            Dimensions::new(4_000, 3_000).expect("dimensions"),
        )),
    );
    let template = Template::parse("${aspect:aspect}").expect("template");
    let (name, _) = template.evaluate(&context, None).expect("evaluation");
    assert_eq!(name.relative_path, "4_3");
}

#[test]
fn template_engine_checks_registered_types_and_hash_prefixes() {
    let mut context = TemplateContext::new();
    context.set(
        VariableId::ContentHash,
        VariableValue::available(TemplateValue::Hash("0123456789abcdef".to_owned())),
    );
    let hash = Template::parse("${content_hash:8}").expect("hash template");
    let (name, _) = hash.evaluate(&context, None).expect("hash evaluation");
    assert_eq!(name.relative_path, "01234567");

    let mut wrong_type = TemplateContext::new();
    wrong_type.set(
        VariableId::Aspect,
        VariableValue::available(TemplateValue::Integer(4)),
    );
    let aspect = Template::parse("${aspect}").expect("aspect template");
    assert!(matches!(
        aspect.evaluate(&wrong_type, None),
        Err(EvaluationError::TypeMismatch {
            id: VariableId::Aspect,
            ..
        })
    ));
}
