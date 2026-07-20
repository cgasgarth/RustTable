use rusttable_export::{
    CapabilitySet, DestinationCapabilityDescriptor, EncoderCapabilityDescriptor, ExportRecipe,
    ExportRecipeDraft, MetadataField, RecipeDestination, RecipeId, RecipeTemplate,
};
use rusttable_image::OutputFormat;

fn draft() -> ExportRecipeDraft {
    let destination = RecipeDestination::new("local", rusttable_export::CollisionPolicy::CreateNew)
        .expect("destination");
    ExportRecipeDraft::new(
        RecipeId::new("shareable").expect("recipe ID"),
        "Shareable",
        "png",
        destination,
        RecipeTemplate::new("default-filename", 1).expect("template"),
    )
}

#[test]
fn recipe_json_round_trip_and_hash_are_canonical() {
    let recipe = ExportRecipe::from_draft(draft()).expect("recipe");
    let json = recipe.canonical_json().expect("canonical JSON");
    let restored = ExportRecipe::from_canonical_json(&json).expect("round trip");
    assert_eq!(json, restored.canonical_json().expect("canonical JSON"));
    assert_eq!(recipe.content_hash(), restored.content_hash());
    assert!(!recipe.export_json().contains("credential_ref"));
}

#[test]
fn recipe_revisions_are_immutable_and_semantic_changes_change_hash() {
    let first = ExportRecipe::from_draft(draft()).expect("recipe");
    let second = first
        .revised(draft().description("different description"))
        .expect("revision");
    assert_eq!(first.revision().get(), 1);
    assert_eq!(second.revision().get(), 2);
    assert_ne!(first.content_hash(), second.content_hash());
}

#[test]
fn capability_negotiation_reports_exact_unsupported_fields() {
    let recipe = ExportRecipe::from_draft(draft().pixel_encoding(
        rusttable_export::PixelEncoding::Integer {
            channels: rusttable_export::ChannelLayout::Rgba,
            depth: rusttable_export::BitDepth::Sixteen,
            color: rusttable_color::ColorEncoding::SrgbD65,
        },
    ))
    .expect("recipe");
    let encoder = EncoderCapabilityDescriptor::new("png")
        .with_format(OutputFormat::Png)
        .with_channel_layout(rusttable_export::ChannelLayout::Rgb)
        .with_bit_depth(rusttable_export::BitDepth::Eight)
        .with_metadata(MetadataField::Exif);
    let capabilities = CapabilitySet::new(
        encoder,
        DestinationCapabilityDescriptor::new("local", false),
    );
    let report = recipe.capability_report(&capabilities).expect("report");
    assert!(!report.is_supported());
    assert!(report.findings().iter().any(|finding| matches!(
        finding,
        rusttable_export::CapabilityFinding::UnsupportedChannels { .. }
    )));
    assert!(report.findings().iter().any(|finding| matches!(
        finding,
        rusttable_export::CapabilityFinding::UnsupportedBitDepth { .. }
    )));
}

#[test]
fn recipe_rejects_secret_material_but_allows_opaque_keyring_reference() {
    let destination =
        RecipeDestination::new("remote", rusttable_export::CollisionPolicy::CreateNew)
            .expect("destination")
            .with_credential_ref("keyring-entry-42");
    let recipe = ExportRecipe::from_draft(ExportRecipeDraft::new(
        RecipeId::new("remote-recipe").expect("recipe ID"),
        "Remote",
        "png",
        destination,
        RecipeTemplate::new("default-filename", 1).expect("template"),
    ))
    .expect("recipe");
    assert!(!recipe.export_json().contains("keyring-entry-42"));
    let secret = ExportRecipe::from_draft(
        draft().encoder_settings(
            rusttable_export::EncoderSettings::new(OutputFormat::Png)
                .with_parameter("password", "do-not-store"),
        ),
    );
    assert!(matches!(
        secret,
        Err(rusttable_export::RecipeError::SecretMaterial { .. })
    ));
}
