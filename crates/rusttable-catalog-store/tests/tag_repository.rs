mod support;

use redb::{Database, ReadableTable, TableDefinition};
use rusttable_catalog::{
    ImportRepository, TagAlias, TagCommand, TagDefinition, TagError, TagId, TagName, TagRepository,
};
use rusttable_catalog_store::{RedbCatalogRepository, RedbTagRepository};
use rusttable_core::{PhotoId, Revision};

const TAG_ALIAS_INDEX: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_tag_alias_index");
const PHOTO_TAG_INDEX: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_photo_tag_index");
const TAG_PHOTO_INDEX: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_tag_photo_index");

fn seed(path: &std::path::Path) {
    let mut catalog = RedbCatalogRepository::open(path).unwrap();
    for (index, source) in ["one.raw", "two.raw"].into_iter().enumerate() {
        catalog
            .commit(&support::record(
                source,
                u128::try_from(index + 1).unwrap(),
                u128::try_from(index + 11).unwrap(),
                u8::try_from(index + 1).unwrap(),
            ))
            .unwrap();
    }
}

fn tag(value: &str, aliases: &[&str]) -> TagDefinition {
    let name = TagName::new(value).unwrap();
    TagDefinition::new(
        TagId::deterministic(None, &name),
        None,
        name,
        aliases.iter().map(|value| TagAlias::new(*value).unwrap()),
    )
    .unwrap()
}

#[test]
fn definitions_and_atomic_assignments_survive_restart() {
    let path = support::temp_path("tags-restart");
    seed(&path);
    let people = tag("People", &["humans"]);
    {
        let mut repository = RedbTagRepository::open(&path).unwrap();
        repository
            .apply(Revision::ZERO, TagCommand::Create(people.clone()))
            .unwrap();
        repository
            .apply(
                Revision::from_u64(1),
                TagCommand::Assign {
                    photo_ids: vec![PhotoId::new(2).unwrap(), PhotoId::new(1).unwrap()],
                    tag_ids: vec![people.id()],
                },
            )
            .unwrap();
    }

    let repository = RedbTagRepository::open(&path).unwrap();
    assert_eq!(repository.resolve("HUMANS").unwrap(), Some(people.id()));
    assert_eq!(
        repository.tags_for_photo(PhotoId::new(1).unwrap()).unwrap(),
        [people.id()]
    );
    assert_eq!(repository.load().unwrap().revision(), Revision::from_u64(2));
    support::remove(&path);
}

#[test]
fn stale_writers_unknown_photos_and_commit_failures_leave_no_partial_changes() {
    let path = support::temp_path("tags-conflicts");
    seed(&path);
    let first = tag("First", &[]);
    let second = tag("Second", &[]);
    let mut left = RedbTagRepository::open(&path).unwrap();
    let mut right = RedbTagRepository::open(&path).unwrap();
    left.apply(Revision::ZERO, TagCommand::Create(first.clone()))
        .unwrap();
    assert!(matches!(
        right.apply(Revision::ZERO, TagCommand::Create(second)),
        Err(TagError::RevisionConflict { actual, .. }) if actual == Revision::from_u64(1)
    ));

    let before = left.load().unwrap();
    assert!(matches!(
        left.apply(
            before.revision(),
            TagCommand::Assign {
                photo_ids: vec![PhotoId::new(1).unwrap(), PhotoId::new(999).unwrap()],
                tag_ids: vec![first.id()],
            },
        ),
        Err(TagError::UnknownPhoto { .. })
    ));
    assert_eq!(left.load().unwrap(), before);

    drop(left);
    drop(right);
    let mut failing =
        RedbTagRepository::open_with_before_commit_hook(&path, || Err(TagError::CommitFailure))
            .unwrap();
    assert_eq!(
        failing.apply(
            Revision::from_u64(1),
            TagCommand::Assign {
                photo_ids: vec![PhotoId::new(1).unwrap()],
                tag_ids: vec![first.id()],
            },
        ),
        Err(TagError::CommitFailure)
    );
    drop(failing);
    assert!(
        RedbTagRepository::open(&path)
            .unwrap()
            .tags_for_photo(PhotoId::new(1).unwrap())
            .unwrap()
            .is_empty()
    );
    support::remove(&path);
}

#[test]
fn derived_alias_and_assignment_indexes_are_rebuildable_from_canonical_state() {
    let path = support::temp_path("tags-rebuild");
    seed(&path);
    let people = tag("People", &["humans"]);
    let mut repository = RedbTagRepository::open(&path).unwrap();
    repository
        .apply(Revision::ZERO, TagCommand::Create(people.clone()))
        .unwrap();
    repository
        .apply(
            Revision::from_u64(1),
            TagCommand::Assign {
                photo_ids: vec![PhotoId::new(1).unwrap()],
                tag_ids: vec![people.id()],
            },
        )
        .unwrap();
    drop(repository);

    let database = Database::open(&path).unwrap();
    let transaction = database.begin_write().unwrap();
    for table_definition in [TAG_ALIAS_INDEX, PHOTO_TAG_INDEX, TAG_PHOTO_INDEX] {
        let mut table = transaction.open_table(table_definition).unwrap();
        let keys = table
            .iter()
            .unwrap()
            .map(|entry| entry.unwrap().0.value().to_vec())
            .collect::<Vec<_>>();
        for key in keys {
            table.remove(key.as_slice()).unwrap();
        }
    }
    transaction.commit().unwrap();
    drop(database);

    let mut repository = RedbTagRepository::open(&path).unwrap();
    assert_eq!(repository.resolve("humans").unwrap(), None);
    assert!(
        repository
            .tags_for_photo(PhotoId::new(1).unwrap())
            .unwrap()
            .is_empty()
    );
    assert!(
        repository
            .photos_with_tag(people.id(), false)
            .unwrap()
            .is_empty()
    );
    let rebuilt = repository.rebuild_indexes().unwrap();
    assert_eq!(rebuilt.aliases, 1);
    assert_eq!(rebuilt.assignments, 1);
    assert_eq!(repository.resolve("humans").unwrap(), Some(people.id()));
    assert_eq!(
        repository.tags_for_photo(PhotoId::new(1).unwrap()).unwrap(),
        [people.id()]
    );
    assert_eq!(
        repository.photos_with_tag(people.id(), false).unwrap(),
        [PhotoId::new(1).unwrap()]
    );
    support::remove(&path);
}
