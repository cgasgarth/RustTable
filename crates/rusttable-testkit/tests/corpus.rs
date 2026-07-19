use std::path::PathBuf;

use rusttable_testkit::corpus::{
    CorpusError, CorpusManifest, CorpusRepository, ExternalStatus, ValidationMode,
};
use rusttable_testkit::fixtures::FixtureManifest;

fn committed_manifests() -> (FixtureManifest, CorpusManifest) {
    let fixtures = FixtureManifest::parse(include_str!("../../../fixtures/manifest.toml"))
        .expect("fixture manifest should parse");
    let corpus = CorpusManifest::parse(include_str!("../../../fixtures/corpus.toml"))
        .expect("corpus manifest should parse");
    (fixtures, corpus)
}

#[test]
fn committed_corpus_is_complete_private_and_offline() {
    let (fixtures, corpus) = committed_manifests();
    corpus
        .validate_against(&fixtures)
        .expect("every corpus matrix reference should be registered");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let report = CorpusRepository::new(root, corpus, fixtures)
        .expect("corpus repository should open")
        .verify(ValidationMode::Local)
        .expect("committed corpus should verify without network access");
    assert!(report.fixture_report().fixtures().len() >= 50);
    assert!(report.external().iter().any(|status| {
        matches!(status, ExternalStatus::Skipped { id, .. } if id == "corpus.large-raw-cache")
    }));
}

#[test]
fn merge_mode_turns_large_asset_skip_into_a_hard_failure() {
    let (fixtures, corpus) = committed_manifests();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let error = CorpusRepository::new(root, corpus, fixtures)
        .expect("corpus repository should open")
        .verify(ValidationMode::Merge)
        .expect_err("merge corpus must contain required large assets");
    assert!(matches!(error, CorpusError::MissingExternal { .. }));
}

#[test]
fn completeness_rejects_unknown_matrix_fixture() {
    let (fixtures, _) = committed_manifests();
    let source = include_str!("../../../fixtures/corpus.toml").replacen(
        "positive_fixture = \"corpus.geometry.portrait\"",
        "positive_fixture = \"missing.positive\"",
        1,
    );
    let corpus = CorpusManifest::parse(&source).expect("shape remains valid");
    assert!(matches!(
        corpus.validate_against(&fixtures),
        Err(CorpusError::UnknownFixture { id, .. }) if id == "missing.positive"
    ));
}
