use rusttable_image::{ImageDimensions, PixelFormat, Roi};

use crate::transfer::TransferRegion;

use super::*;

fn dimensions() -> ImageDimensions {
    ImageDimensions::new(17, 11).expect("dimensions")
}

fn budget(bytes: u64) -> TileMemoryBudget {
    TileMemoryBudget::new(bytes, 32, bytes, 0).expect("budget")
}

fn request(preferred: [u32; 2], budget_bytes: u64) -> GpuTileRequest {
    let generation = DeviceGeneration::new(4);
    let class = ResourceClass::buffer(generation, 1, 1).with_alignment(4);
    let input = TileResourceSpec::new("input", class, TileArea::Input, 4).expect("input");
    let output = TileResourceSpec::new("output", class, TileArea::Output, 4).expect("output");
    GpuTileRequest::new(
        generation,
        dimensions(),
        Roi::full(dimensions()),
        preferred,
        [2, 2],
        [17, 11],
        EdgeOverlap::uniform(2),
        TileAlignment::new(1, 1, 2, 2).expect("alignment"),
        budget(budget_bytes),
        vec![input, output],
    )
    .expect("request")
}

#[test]
fn candidate_is_deterministic_and_row_major() {
    let first = GpuTilePlanner::plan(&request([6, 4], 500)).expect("plan");
    let second = GpuTilePlanner::plan(&request([6, 4], 500)).expect("plan");
    assert_eq!(first.receipt.identity, second.receipt.identity);
    let candidate = first.candidate(0).expect("candidate");
    assert_eq!(candidate.tiles[0].output.x, 0);
    assert_eq!(candidate.tiles[0].output.y, 0);
    assert_eq!(candidate.tiles[1].output.y, 0);
    assert_eq!(candidate.tiles[3].output.x, 0);
    assert_eq!(candidate.tiles[3].output.y, 4);
    candidate.validate_coverage().expect("exact coverage");
}

#[test]
fn overlap_is_clipped_at_legal_host_boundaries() {
    let plan = GpuTilePlanner::plan(&request([6, 4], 500)).expect("plan");
    let first = plan
        .candidate(0)
        .expect("candidate")
        .tiles
        .first()
        .expect("tile");
    assert_eq!(first.input_roi, Roi::new(0, 0, 8, 6).expect("roi"));
    let last = plan
        .candidate(0)
        .expect("candidate")
        .tiles
        .last()
        .expect("tile");
    assert_eq!(last.input_roi.right(), 17);
    assert_eq!(last.input_roi.bottom(), 11);
}

#[test]
fn budget_selects_strictly_smaller_bounded_candidates() {
    let plan = GpuTilePlanner::plan(&request([16, 10], 1_500)).expect("plan");
    assert!(plan.candidates.len() <= 3);
    for pair in plan.candidates.windows(2) {
        assert!(pair[1].width < pair[0].width || pair[1].height < pair[0].height);
        assert!(pair[1].memory.required_bytes <= pair[1].memory.available_bytes);
    }
}

#[test]
fn coverage_rejects_overlap_and_gap() {
    let roi = Roi::new(0, 0, 4, 4).expect("roi");
    let mut overlap = CoverageModel::new(roi);
    overlap
        .add(Tile::new(0, 0, 3, 4).expect("tile"))
        .expect("tile");
    assert_eq!(
        overlap.add(Tile::new(2, 0, 2, 4).expect("tile")),
        Err(CoverageError::Overlap)
    );

    let mut gap = CoverageModel::new(roi);
    gap.add(Tile::new(0, 0, 2, 4).expect("tile")).expect("tile");
    assert_eq!(gap.validate(), Err(CoverageError::Gap));
}

#[test]
fn resident_intermediates_are_generation_local_and_compatible_only_at_boundary() {
    let generation = DeviceGeneration::new(8);
    let class = ResourceClass::texture(generation, [4, 4, 1], ResourceFormat::Rgba16Float, 1, 1, 1)
        .with_size(128);
    let first = ResidentIntermediate::new(
        1,
        Roi::new(0, 0, 4, 4).expect("roi"),
        class,
        128,
        Lifetime::new(1, 2).expect("life"),
        9,
    )
    .expect("intermediate");
    let next = ResidentIntermediate::new(
        2,
        first.roi,
        class,
        128,
        Lifetime::new(2, 3).expect("life"),
        9,
    )
    .expect("intermediate");
    assert!(first.compatible_with(&next));
}

#[test]
fn host_boundary_uses_transfer_legality() {
    let dims = ImageDimensions::new(8, 4).expect("dimensions");
    let format = PixelFormat::rgba8();
    let boundary = HostBoundary::new(
        1,
        2,
        dims,
        TransferRegion::new(0, 0, 8, 4).expect("region"),
        32,
        format,
        128,
    )
    .expect("boundary");
    assert_eq!(boundary.descriptor.region.width, 8);
}
