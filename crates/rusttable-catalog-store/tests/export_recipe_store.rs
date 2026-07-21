use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use rusttable_catalog_store::{CURRENT_SCHEMA_VERSION, RecipeStoreError, RedbRecipeRepository};
use rusttable_export::{
    ExportRecipe, ExportRecipeDraft, ImportConflictPolicy, RecipeDestination, RecipeId,
    RecipeTemplate,
};

static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

fn path() -> PathBuf {
    let suffix = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "rusttable-export-recipes-{}-{suffix}.redb",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    path
}

fn recipe(id: &str) -> ExportRecipe {
    let destination = RecipeDestination::new(id, rusttable_export::CollisionPolicy::CreateNew)
        .expect("destination");
    ExportRecipe::from_draft(ExportRecipeDraft::new(
        RecipeId::new(id).expect("recipe ID"),
        "Stored recipe",
        "png",
        destination,
        RecipeTemplate::new("default-filename", 1).expect("template"),
    ))
    .expect("recipe")
}

#[test]
fn recipe_store_migrates_crud_revisions_and_reference_deletion() {
    assert_eq!(CURRENT_SCHEMA_VERSION, 8);
    let path = path();
    let repository = RedbRecipeRepository::open(&path).expect("open");
    let first = recipe("stored");
    repository.create(&first).expect("create");
    let second = first
        .revised(ExportRecipeDraft::new(
            RecipeId::new("stored").expect("recipe ID"),
            "Changed",
            "png",
            RecipeDestination::new("stored", rusttable_export::CollisionPolicy::CreateNew)
                .expect("destination"),
            RecipeTemplate::new("default-filename", 1).expect("template"),
        ))
        .expect("revision");
    repository
        .update(first.revision(), &second)
        .expect("update");
    assert_eq!(
        repository.find(second.id(), None).unwrap().unwrap().name(),
        "Changed"
    );
    assert_eq!(
        repository
            .find(second.id(), Some(first.revision()))
            .unwrap()
            .unwrap()
            .revision(),
        first.revision()
    );

    repository
        .mark_reference(second.id(), "queued-job-1")
        .expect("reference");
    assert_eq!(
        repository.delete(second.id()),
        Err(RecipeStoreError::Referenced)
    );
    let disabled = repository
        .disable(second.revision(), second.id())
        .expect("disable");
    assert!(!disabled.enabled());

    let imported = repository
        .import_json(
            &second.canonical_json().unwrap(),
            ImportConflictPolicy::CreateNewId,
        )
        .expect("import clone");
    assert_ne!(imported.id(), second.id());
    let _ = std::fs::remove_file(path);
}

#[test]
fn recipe_store_protects_built_ins_and_unknown_references() {
    let path = path();
    let repository = RedbRecipeRepository::open(&path).expect("open");
    let built_in = ExportRecipe::from_draft(
        ExportRecipeDraft::new(
            RecipeId::new("builtin").expect("recipe ID"),
            "Built-in",
            "png",
            RecipeDestination::new("builtin", rusttable_export::CollisionPolicy::CreateNew)
                .expect("destination"),
            RecipeTemplate::new("default-filename", 1).expect("template"),
        )
        .built_in(true),
    )
    .expect("recipe");
    repository.create(&built_in).expect("create");
    let revised = built_in
        .revised(ExportRecipeDraft::new(
            RecipeId::new("builtin").expect("recipe ID"),
            "Changed",
            "png",
            RecipeDestination::new("builtin", rusttable_export::CollisionPolicy::CreateNew)
                .expect("destination"),
            RecipeTemplate::new("default-filename", 1).expect("template"),
        ))
        .expect("revision");
    assert_eq!(
        repository.update(built_in.revision(), &revised),
        Err(RecipeStoreError::BuiltInImmutable)
    );
    assert_eq!(
        repository.mark_reference(&RecipeId::new("missing").expect("recipe ID"), "ref"),
        Err(RecipeStoreError::Conflict)
    );
    let _ = std::fs::remove_file(path);
}
