mod support;

use rusttable_catalog::ImportRepository;
use rusttable_catalog_store::RedbImportRepository;

#[test]
fn commit_lookup_close_reopen_and_list_are_canonical() {
    let path = support::temp_path("round-trip");
    let first = support::record("zeta/photo.raw", 1, 2, 7);
    let second = support::record("alpha/photo.raw", 3, 4, 8);
    {
        let mut repository = RedbImportRepository::open(&path).expect("open store");
        repository.commit(&first).expect("first commit");
        repository.commit(&second).expect("second commit");
        assert_eq!(
            repository.find_by_source(first.source()).unwrap(),
            Some(first.clone())
        );
        assert_eq!(
            repository.find_by_photo_id(first.photo().id()).unwrap(),
            Some(first.clone())
        );
        assert_eq!(
            repository
                .find_by_asset_id(first.photo().primary_asset_id())
                .unwrap(),
            Some(first.clone())
        );
        assert_eq!(
            repository
                .list()
                .unwrap()
                .iter()
                .map(|record| record.source().as_str())
                .collect::<Vec<_>>(),
            ["alpha/photo.raw", "zeta/photo.raw"]
        );
    }
    let repository = RedbImportRepository::open(&path).expect("reopen store");
    assert_eq!(
        repository.find_by_source(second.source()).unwrap(),
        Some(second)
    );
    assert_eq!(repository.list().unwrap().len(), 2);
    support::remove(&path);
}
