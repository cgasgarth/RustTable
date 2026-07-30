use rusttable_core::template::{Template, TemplateContext, VariableId};
use rusttable_export::{ArtifactKind, DestinationCapabilities, ExportPlan, ExportRequest};

#[test]
fn export_plan_uses_one_engine_for_images_sidecars_and_bundle_members() {
    let mut context = TemplateContext::new();
    context.set_text(VariableId::SourceStem, "photo");
    let template = Template::parse("exports/${source_stem}").expect("template");
    let requests = [
        ExportRequest::new(ArtifactKind::Image, template.clone(), context.clone()),
        ExportRequest::new(ArtifactKind::Sidecar, template.clone(), context.clone()),
        ExportRequest::new(ArtifactKind::BundleMember, template, context),
    ];
    let plan = ExportPlan::build(&requests).expect("plan");
    assert_eq!(plan.artifacts.len(), 3);
    assert_eq!(plan.collision_groups[0].artifact_indexes, vec![0, 1, 2]);
    assert_eq!(plan.artifacts[1].name.relative_path, "exports/photo");
    assert_eq!(plan.receipt_hash.len(), 64);
}

#[test]
fn export_plan_reports_destination_capability_collisions_without_resolving_them() {
    let mut first = TemplateContext::new();
    first.set_text(VariableId::SourceStem, "A");
    let mut second = TemplateContext::new();
    second.set_text(VariableId::SourceStem, "a");
    let template = Template::parse("${source_stem}").expect("template");
    let requests = [
        ExportRequest::new(ArtifactKind::Image, template.clone(), first),
        ExportRequest::new(ArtifactKind::Image, template, second),
    ];
    let plan = ExportPlan::build_with_capabilities(
        &requests,
        DestinationCapabilities {
            case_sensitive: false,
            unicode_normalized: true,
        },
    )
    .expect("plan");
    assert_eq!(plan.collision_groups.len(), 1);
    assert_eq!(plan.artifacts[0].name.relative_path, "A");
    assert_eq!(plan.artifacts[1].name.relative_path, "a");
}
