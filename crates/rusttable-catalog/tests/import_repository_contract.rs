use rusttable_catalog::{ImportRecord, ImportRepository, RepositoryError, SourcePath};
use rusttable_core::{AssetId, PhotoId};

struct EmptyRepository;

impl ImportRepository for EmptyRepository {
    fn find_by_source(
        &self,
        _source: &SourcePath,
    ) -> Result<Option<ImportRecord>, RepositoryError> {
        Ok(None)
    }

    fn find_by_photo_id(
        &self,
        _photo_id: PhotoId,
    ) -> Result<Option<ImportRecord>, RepositoryError> {
        Ok(None)
    }

    fn find_by_asset_id(
        &self,
        _asset_id: AssetId,
    ) -> Result<Option<ImportRecord>, RepositoryError> {
        Ok(None)
    }

    fn commit(&mut self, _record: &ImportRecord) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn list(&self) -> Result<Vec<ImportRecord>, RepositoryError> {
        Ok(Vec::new())
    }
}

#[test]
fn repository_port_is_object_safe_and_returns_owned_rusttable_values() {
    let repository: Box<dyn ImportRepository> = Box::new(EmptyRepository);

    assert_eq!(repository.list().expect("empty repository"), []);
    assert!(
        repository
            .find_by_source(&SourcePath::new("one.raw").expect("valid source"))
            .expect("lookup")
            .is_none()
    );
}
