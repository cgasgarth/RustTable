mod support;

use rusttable_catalog::{
    DurableHistoryError, DurableHistoryService, HistoryCommand, HistoryOperationKind,
    HistoryOperationSummary, HistoryPayload, HistoryRepository, HistoryRepositoryError,
    HistoryState,
};
use rusttable_catalog_store::RedbHistoryRepository;
use rusttable_core::{Edit, EditId, PhotoId, Revision};

fn payload(id: u128) -> HistoryPayload {
    HistoryPayload::new(
        Edit::new(
            EditId::new(id).unwrap(),
            PhotoId::new(7).unwrap(),
            Revision::ZERO,
            [],
        )
        .unwrap(),
        [u8::try_from(id & 0xff).unwrap(), 0xa5],
        id.to_be_bytes().to_vec(),
        HistoryOperationSummary::new(HistoryOperationKind::Parameter, None, None, "parameter")
            .unwrap(),
    )
}

#[test]
fn history_payloads_and_pointers_recover_exactly_after_restart() {
    let path = support::temp_path("history-restart");
    let photo = PhotoId::new(7).unwrap();
    let mut state = HistoryState::new(photo);
    {
        let mut repository = RedbHistoryRepository::open(&path, photo).unwrap();
        let expected = state.version();
        DurableHistoryService::apply(
            &mut state,
            expected,
            HistoryCommand::Append {
                payload: payload(1),
            },
            &mut repository,
        )
        .unwrap();
        let stored = repository.load().unwrap().unwrap();
        let current = stored.current_revision().unwrap();
        assert_eq!(current.payload().edit().id().get(), 1);
        assert_eq!(current.payload().mask_bytes(), &[1, 0xa5]);
        assert_eq!(current.payload().pipeline_bytes(), 1_u128.to_be_bytes());
    }
    let repository = RedbHistoryRepository::open(&path, photo).unwrap();
    let recovered = repository.load().unwrap().unwrap();
    assert_eq!(recovered.active_cursor(), state.active_cursor());
    assert_eq!(recovered.version(), state.version());
    assert_eq!(recovered.current_revision(), state.current_revision());
    support::remove(&path);
}

#[test]
fn stale_writer_is_rejected_and_reload_is_deterministic() {
    let path = support::temp_path("history-stale");
    let photo = PhotoId::new(7).unwrap();
    let mut repository = RedbHistoryRepository::open(&path, photo).unwrap();
    let mut authoritative = HistoryState::new(photo);
    let expected = authoritative.version();
    DurableHistoryService::apply(
        &mut authoritative,
        expected,
        HistoryCommand::Append {
            payload: payload(1),
        },
        &mut repository,
    )
    .unwrap();

    let mut stale = HistoryState::new(photo);
    let expected = stale.version();
    let error = DurableHistoryService::apply(
        &mut stale,
        expected,
        HistoryCommand::Append {
            payload: payload(2),
        },
        &mut repository,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        DurableHistoryError::Repository(HistoryRepositoryError::VersionConflict { .. })
    ));
    assert_eq!(stale.version().get(), 0);
    let recovered = repository.load().unwrap().unwrap();
    assert_eq!(
        recovered
            .current_revision()
            .unwrap()
            .payload()
            .edit()
            .id()
            .get(),
        1
    );
    assert_eq!(recovered.revisions().count(), 1);
    support::remove(&path);
}
