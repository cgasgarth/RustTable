mod support;

use rusttable_catalog::ImportRepository;
use rusttable_catalog_store::RedbImportRepository;

#[test]
fn equal_logical_records_round_trip_independently_of_insert_order() {
    let first_path = support::temp_path("canonical-first");
    let second_path = support::temp_path("canonical-second");
    let records = [
        support::record("b.raw", 1, 2, 1),
        support::record("a.raw", 3, 4, 2),
    ];
    {
        let mut repository = RedbImportRepository::open(&first_path).unwrap();
        for record in &records {
            repository.commit(record).unwrap();
        }
    }
    {
        let mut repository = RedbImportRepository::open(&second_path).unwrap();
        for record in records.iter().rev() {
            repository.commit(record).unwrap();
        }
    }
    let first = RedbImportRepository::open(&first_path)
        .unwrap()
        .list()
        .unwrap();
    let second = RedbImportRepository::open(&second_path)
        .unwrap()
        .list()
        .unwrap();
    assert_eq!(first, second);
    support::remove(&first_path);
    support::remove(&second_path);
}
