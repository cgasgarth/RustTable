use super::helpers::fixture_hash;
use super::{BenchmarkScenario, ScenarioState};

#[must_use]
pub fn initial_scenarios() -> Vec<BenchmarkScenario> {
    SCENARIOS.into_iter().map(build_scenario).collect()
}

#[derive(Clone, Copy)]
struct ScenarioSpec(
    &'static str,
    ScenarioState,
    &'static str,
    &'static str,
    u32,
    u32,
    bool,
    Option<u32>,
);

const SCENARIOS: [ScenarioSpec; 10] = [
    ScenarioSpec(
        "catalog-open-checkpoint",
        ScenarioState::InactivePlaceholder,
        "corpus.compat.library-schema",
        "catalog.open",
        256,
        256,
        false,
        Some(181),
    ),
    ScenarioSpec(
        "import-registration",
        ScenarioState::InactivePlaceholder,
        "corpus.compat.library-schema",
        "import.register",
        256,
        256,
        false,
        Some(256),
    ),
    ScenarioSpec(
        "raster-decode",
        ScenarioState::InactivePlaceholder,
        "corpus.raster.png.16-alpha",
        "decode.raster",
        4,
        3,
        false,
        Some(226),
    ),
    ScenarioSpec(
        "raw-decode",
        ScenarioState::InactivePlaceholder,
        "corpus.raw.bayer.12-2row",
        "decode.raw",
        4,
        3,
        false,
        Some(233),
    ),
    ScenarioSpec(
        "thumbnail-generation",
        ScenarioState::InactivePlaceholder,
        "corpus.raster.png.16-alpha",
        "thumbnail.generate",
        4,
        3,
        false,
        Some(253),
    ),
    ScenarioSpec(
        "minimal-cpu-pipeline",
        ScenarioState::InactivePlaceholder,
        "corpus.raster.png.16-alpha",
        "pipeline.cpu",
        4,
        3,
        false,
        Some(266),
    ),
    ScenarioSpec(
        "minimal-wgpu-pipeline",
        ScenarioState::Qualification,
        "corpus.raster.png.16-alpha",
        "pipeline.wgpu",
        2048,
        1365,
        true,
        Some(301),
    ),
    ScenarioSpec(
        "preview-update",
        ScenarioState::InactivePlaceholder,
        "corpus.raster.png.16-alpha",
        "preview.update",
        4,
        3,
        false,
        Some(181),
    ),
    ScenarioSpec(
        "full-export",
        ScenarioState::InactivePlaceholder,
        "corpus.raster.png.16-alpha",
        "export.full",
        4,
        3,
        false,
        Some(470),
    ),
    ScenarioSpec(
        "library-projection-10k",
        ScenarioState::InactivePlaceholder,
        "corpus.compat.library-schema",
        "library.project",
        10_000,
        1,
        false,
        Some(213),
    ),
];

fn build_scenario(spec: ScenarioSpec) -> BenchmarkScenario {
    BenchmarkScenario {
        id: spec.0.to_owned(),
        state: spec.1,
        fixture_ids: vec![spec.2.to_owned()],
        fixture_content_sha256: vec![fixture_hash(spec.2)],
        width: spec.4,
        height: spec.5,
        operations: vec![spec.3.to_owned()],
        thread_count: 1,
        memory_cap_bytes: 512 * 1024 * 1024,
        timeout_ms: 30_000,
        expected_output_sha256: None,
        warmup_iterations: 2,
        repetitions: 5,
        requires_gpu: spec.6,
        blocking_issue: spec.7,
    }
}
