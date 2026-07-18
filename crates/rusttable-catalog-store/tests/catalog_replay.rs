mod support;

use rusttable_catalog::{CatalogCommand, CatalogState, ImportRepository};
use rusttable_catalog_store::RedbImportRepository;
use rusttable_core::PhotoId;

#[test]
fn listed_records_replay_to_equal_catalog_state() {
    let path = support::temp_path("replay");
    let records = [
        support::record("b.raw", 1, 2, 1),
        support::record("a.raw", 3, 4, 2),
    ];
    let mut repository = RedbImportRepository::open(&path).unwrap();
    for record in records {
        repository.commit(&record).unwrap();
    }
    let mut state = CatalogState::new();
    for record in repository.list().unwrap() {
        let revision = state.revision();
        state
            .apply(
                revision,
                CatalogCommand::RegisterPhoto(record.photo().clone()),
            )
            .unwrap();
    }
    assert_eq!(state.revision().get(), 2);
    assert_eq!(
        state
            .photos()
            .map(rusttable_core::Photo::id)
            .collect::<Vec<_>>(),
        [PhotoId::new(1).unwrap(), PhotoId::new(3).unwrap()]
    );
    support::remove(&path);
}
