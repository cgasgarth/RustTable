use rusttable_catalog::{
    MAX_TAG_ALIASES, MAX_TAG_NAME_BYTES, TagAlias, TagCommand, TagDefinition, TagError, TagId,
    TagName, TagState,
};
use rusttable_core::{PhotoId, Revision};

fn name(value: &str) -> TagName {
    TagName::new(value).expect("valid tag name")
}

fn alias(value: &str) -> TagAlias {
    TagAlias::new(value).expect("valid tag alias")
}

fn definition(parent_id: Option<TagId>, value: &str, aliases: &[&str]) -> TagDefinition {
    let name = name(value);
    TagDefinition::new(
        TagId::deterministic(parent_id, &name),
        parent_id,
        name,
        aliases.iter().map(|value| alias(value)),
    )
    .expect("valid tag definition")
}

fn create(state: &mut TagState, definition: TagDefinition) {
    state
        .apply(state.revision(), TagCommand::Create(definition))
        .expect("create tag");
}

#[test]
fn canonical_hierarchy_and_aliases_normalize_for_resolution() {
    let mut state = TagState::new();
    let places = definition(None, "  PlＡces  ", &[]);
    let france = definition(
        Some(places.id()),
        " France ",
        &["  EUROPE | FRANCE  ", "Hexagone"],
    );
    create(&mut state, places.clone());
    create(&mut state, france.clone());

    assert_eq!(places.name().canonical(), "places");
    assert_eq!(state.canonical_path(france.id()), Some("places|france"));
    assert_eq!(
        state.resolve(" PLACES | FRANCE ").map(TagDefinition::id),
        Some(france.id())
    );
    assert_eq!(
        state.resolve("hexagone").map(TagDefinition::id),
        Some(france.id())
    );
    assert_eq!(
        state
            .children(Some(places.id()))
            .map(TagDefinition::id)
            .collect::<Vec<_>>(),
        [france.id()]
    );
}

#[test]
fn identities_and_projection_order_are_deterministic() {
    let normalized = name("Ｐeople");
    assert_eq!(
        TagId::deterministic(None, &normalized),
        TagId::deterministic(None, &name("people"))
    );

    let mut state = TagState::new();
    let wildlife = definition(None, "Wildlife", &["animals"]);
    let people = definition(None, "People", &["humans"]);
    create(&mut state, wildlife.clone());
    create(&mut state, people.clone());

    assert_eq!(
        state
            .projections()
            .into_iter()
            .map(|projection| projection.canonical_path)
            .collect::<Vec<_>>(),
        ["people", "wildlife"]
    );

    let renamed =
        TagDefinition::new(people.id(), None, name("Portraits"), [alias("humans")]).unwrap();
    state
        .apply(state.revision(), TagCommand::Update(renamed))
        .unwrap();
    assert_eq!(
        state.resolve("portraits").map(TagDefinition::id),
        Some(people.id())
    );
}

#[test]
fn migration_import_preserves_explicit_stable_ids_and_assignments() {
    let imported_id = TagId::new(42).unwrap();
    let imported =
        TagDefinition::new(imported_id, None, name("Imported"), [alias("legacy")]).unwrap();
    let photo_id = PhotoId::new(7).unwrap();

    let state = TagState::import([imported], [(photo_id, imported_id)]).unwrap();

    assert_eq!(
        state.resolve("legacy").map(TagDefinition::id),
        Some(imported_id)
    );
    assert_eq!(
        state
            .tags_for_photo(photo_id)
            .map(TagDefinition::id)
            .collect::<Vec<_>>(),
        [imported_id]
    );
    assert_eq!(state.revision(), Revision::ZERO);
}

#[test]
fn hierarchy_cycles_and_normalized_name_or_alias_conflicts_are_atomic() {
    let mut state = TagState::new();
    let parent = definition(None, "Places", &[]);
    let child = definition(Some(parent.id()), "France", &["hexagone"]);
    create(&mut state, parent.clone());
    create(&mut state, child.clone());

    let before = state.clone();
    let cycle =
        TagDefinition::new(parent.id(), Some(child.id()), parent.name().clone(), []).unwrap();
    assert!(matches!(
        state.apply(state.revision(), TagCommand::Update(cycle)),
        Err(TagError::HierarchyCycle { .. })
    ));
    assert_eq!(state, before);

    let conflicting = TagDefinition::new(
        TagId::new(99).unwrap(),
        Some(parent.id()),
        name("ＦＲＡＮＣＥ"),
        [],
    )
    .unwrap();
    assert!(matches!(
        state.apply(state.revision(), TagCommand::Create(conflicting)),
        Err(TagError::CanonicalPathConflict { .. })
    ));
    assert_eq!(state, before);

    let alias_conflict = TagDefinition::new(
        TagId::new(100).unwrap(),
        None,
        name("Country"),
        [alias("places|france")],
    )
    .unwrap();
    assert!(matches!(
        state.apply(state.revision(), TagCommand::Create(alias_conflict)),
        Err(TagError::AliasConflict { .. })
    ));
    assert_eq!(state, before);
}

#[test]
fn photo_assignment_and_removal_are_atomic_and_ordered() {
    let mut state = TagState::new();
    let people = definition(None, "People", &[]);
    let family = definition(Some(people.id()), "Family", &[]);
    create(&mut state, people.clone());
    create(&mut state, family.clone());
    let first = PhotoId::new(1).unwrap();
    let second = PhotoId::new(2).unwrap();

    state
        .apply(
            state.revision(),
            TagCommand::Assign {
                photo_ids: vec![second, first],
                tag_ids: vec![family.id(), people.id()],
            },
        )
        .unwrap();
    assert_eq!(
        state
            .tags_for_photo(first)
            .map(TagDefinition::id)
            .collect::<Vec<_>>(),
        [people.id(), family.id()]
    );
    assert_eq!(
        state.photos_with_tag(people.id(), true).unwrap(),
        [first, second]
    );

    let before = state.clone();
    assert!(matches!(
        state.apply(
            state.revision(),
            TagCommand::Remove {
                photo_ids: vec![first, second],
                tag_ids: vec![family.id(), TagId::new(777).unwrap()],
            },
        ),
        Err(TagError::UnknownTag { .. })
    ));
    assert_eq!(state, before);

    state
        .apply(
            state.revision(),
            TagCommand::Remove {
                photo_ids: vec![first, second],
                tag_ids: vec![family.id()],
            },
        )
        .unwrap();
    assert_eq!(
        state
            .tags_for_photo(first)
            .map(TagDefinition::id)
            .collect::<Vec<_>>(),
        [people.id()]
    );
}

#[test]
fn stale_revisions_and_bounded_invalid_input_do_not_mutate_state() {
    assert!(matches!(
        TagName::new("x".repeat(MAX_TAG_NAME_BYTES + 1)),
        Err(TagError::NameTooLong)
    ));
    assert!(matches!(
        TagName::new("places|france"),
        Err(TagError::HierarchySeparator)
    ));
    assert!(matches!(
        TagAlias::new("places||france"),
        Err(TagError::EmptyPathSegment)
    ));

    let tag_name = name("bounded");
    let excessive_aliases = (0..=MAX_TAG_ALIASES)
        .map(|index| alias(&format!("alias-{index}")))
        .collect::<Vec<_>>();
    assert!(matches!(
        TagDefinition::new(
            TagId::deterministic(None, &tag_name),
            None,
            tag_name,
            excessive_aliases,
        ),
        Err(TagError::TooManyAliases)
    ));

    let mut state = TagState::new();
    let before = state.clone();
    let error = state
        .apply(
            Revision::from_u64(1),
            TagCommand::Create(definition(None, "one", &[])),
        )
        .unwrap_err();
    assert_eq!(
        error,
        TagError::RevisionConflict {
            expected: Revision::from_u64(1),
            actual: Revision::ZERO,
        }
    );
    assert_eq!(state, before);
}
