use rusttable_pixelpipe::{
    AreaSource, BackendRequirement, BufferRequirement, BytesPerPixel, EdgeOverlap,
    FullFrameRequirements, MemoryBudget, MemoryFactor, NodeRequirement, NodeRequirements,
    ResourceKind, RoiChain, RoiStage, ScaleRatio, TileAlignment, TileBackend, TileDeviceLimits,
    TileDimensions, TilePlanError, TilePlanRequest, TilePlanner, TileRect,
};

const HARD_BUDGET: u64 = 64 * 1024 * 1024;

fn chain(output: TileRect, overlap: EdgeOverlap) -> RoiChain {
    RoiChain::new(
        output,
        output,
        vec![
            RoiStage::new(
                7,
                output,
                output,
                ScaleRatio::new(1, 1).expect("scale"),
                overlap,
            )
            .expect("stage"),
        ],
    )
    .expect("chain")
}

fn backend(temporary_factor: MemoryFactor) -> BackendRequirement {
    let bytes = BytesPerPixel::new(4).expect("bytes per pixel");
    let alignment = TileAlignment::new(1, 1, 1, 1).expect("alignment");
    let dimensions = TileDimensions::new(1, 1, 4, 4, 64, 64).expect("dimensions");
    BackendRequirement::new(
        BufferRequirement::new(
            ResourceKind::Input,
            bytes,
            MemoryFactor::one(),
            AreaSource::Input,
        ),
        BufferRequirement::new(
            ResourceKind::Output,
            bytes,
            MemoryFactor::one(),
            AreaSource::Output,
        ),
        BufferRequirement::new(
            ResourceKind::Temporary,
            bytes,
            temporary_factor,
            AreaSource::Larger,
        ),
        128,
        EdgeOverlap::uniform(1),
        alignment,
        dimensions,
    )
}

fn requirements(
    output: TileRect,
    overlap: EdgeOverlap,
    cpu: BackendRequirement,
    gpu: Option<BackendRequirement>,
) -> NodeRequirements {
    NodeRequirements::new(
        vec![NodeRequirement::new(7, "exposure#7", cpu, gpu).expect("node")],
        0..1,
        chain(output, overlap),
    )
    .expect("requirements")
}

fn request(output: TileRect, backend: TileBackend) -> TilePlanRequest {
    TilePlanRequest::new(
        output,
        TileDeviceLimits::new(64, 64).expect("device"),
        MemoryBudget::new(backend, HARD_BUDGET, HARD_BUDGET).expect("budget"),
    )
}

#[test]
fn grid_is_row_major_exact_cover_and_clips_overlap_at_source_edges() {
    let output = TileRect::new(0, 0, 13, 9);
    let requirements = requirements(
        output,
        EdgeOverlap::uniform(2),
        backend(MemoryFactor::one()),
        None,
    );
    let plan = TilePlanner
        .plan(&requirements, request(output, TileBackend::Cpu))
        .expect("plan");

    assert_eq!(plan.grid().columns(), 4);
    assert_eq!(plan.grid().rows(), 3);
    assert_eq!(plan.grid().tiles().len(), 12);
    assert_eq!(plan.grid().tiles()[0].output(), TileRect::new(0, 0, 4, 4));
    assert_eq!(
        plan.grid().tiles()[0].input_rois()[0].rect(),
        TileRect::new(0, 0, 7, 7)
    );
    assert_eq!(
        plan.grid().tiles().last().expect("last tile").output(),
        TileRect::new(12, 8, 1, 1)
    );

    let mut covered = [0_u8; 13 * 9];
    for tile in plan.grid().tiles() {
        for y in tile.output().y()..tile.output().end_y().expect("end y") {
            for x in tile.output().x()..tile.output().end_x().expect("end x") {
                let index = usize::try_from(y * 13 + x).expect("index");
                covered[index] = covered[index].saturating_add(1);
            }
        }
    }
    assert!(covered.iter().all(|count| *count == 1));
}

#[test]
fn scale_chain_maps_output_edges_backwards_and_clips_to_input() {
    let input = TileRect::new(0, 0, 10, 6);
    let intermediate = TileRect::new(0, 0, 5, 3);
    let output = TileRect::new(0, 0, 5, 3);
    let chain = RoiChain::new(
        input,
        output,
        vec![
            RoiStage::new(
                7,
                input,
                intermediate,
                ScaleRatio::new(2, 1).expect("scale"),
                EdgeOverlap::uniform(1),
            )
            .expect("first stage"),
            RoiStage::new(
                8,
                intermediate,
                output,
                ScaleRatio::new(1, 1).expect("scale"),
                EdgeOverlap::uniform(1),
            )
            .expect("second stage"),
        ],
    )
    .expect("chain");
    let rois = chain
        .required_inputs(TileRect::new(4, 2, 1, 1))
        .expect("ROIs");

    assert_eq!(rois[1].1, TileRect::new(3, 1, 2, 2));
    assert_eq!(rois[0].1, TileRect::new(5, 1, 5, 5));
}

#[test]
fn cpu_and_gpu_budgets_are_independent() {
    let output = TileRect::new(0, 0, 32, 32);
    let requirements = requirements(
        output,
        EdgeOverlap::default(),
        backend(MemoryFactor::one()),
        Some(backend(
            MemoryFactor::rational(10_000_000, 1).expect("factor"),
        )),
    );
    let planner = TilePlanner;

    assert!(
        !planner
            .plan(&requirements, request(output, TileBackend::Cpu))
            .expect("CPU plan")
            .full_frame()
    );
    assert!(matches!(
        planner.plan(&requirements, request(output, TileBackend::Gpu)),
        Err(TilePlanError::MinimumTileDoesNotFit { .. })
    ));
}

#[test]
fn full_frame_is_not_subdivided_and_unknown_resources_block_before_grid() {
    let output = TileRect::new(0, 0, 13, 9);
    let full_frame_backend = backend(MemoryFactor::one())
        .with_full_frame(FullFrameRequirements::new(true, false, false));
    let full_frame = TilePlanner
        .plan(
            &requirements(output, EdgeOverlap::default(), full_frame_backend, None),
            request(output, TileBackend::Cpu),
        )
        .expect("full-frame plan");
    assert!(full_frame.full_frame());
    assert_eq!(full_frame.grid().tiles().len(), 1);

    assert!(matches!(
        TilePlanner.plan(
            &requirements(
                output,
                EdgeOverlap::default(),
                backend(MemoryFactor::Unknown),
                None
            ),
            request(output, TileBackend::Cpu)
        ),
        Err(TilePlanError::UnknownRequiredResource { .. })
    ));
}

#[test]
fn empty_output_is_explicit_and_has_no_live_components() {
    let plan = TilePlanner
        .plan(
            &requirements(
                TileRect::new(0, 0, 1, 1),
                EdgeOverlap::default(),
                backend(MemoryFactor::one()),
                None,
            ),
            request(TileRect::new(0, 0, 0, 0), TileBackend::Cpu).allow_empty(),
        )
        .expect("empty plan");
    assert!(plan.grid().tiles().is_empty());
    assert_eq!(plan.estimate().total_bytes(), 0);
    assert!(plan.estimate().components().is_empty());
}
