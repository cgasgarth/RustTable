use std::cell::Cell;

use rusttable_color::{ColorEncoding, Precision};
use rusttable_image::{ImageDimensions, Orientation, PixelFormat, Roi};
use rusttable_pixelpipe::{
    Background, ColorIdentity, DescriptorPreparationSource, ImplementationIdentity,
    OperationPreparationSource, OutputSpec, PipelineGeneration, PipelinePreparer, PipelinePurpose,
    PipelineSnapshot, PipelineSnapshotInput, PreparationContext, PreparationError,
    PreparationSourceError, PreparedOperation, ResourceMetadata, SourceDescriptor, SourceIdentity,
};
use rusttable_processing::descriptor::{OperationDescriptor, exposure_descriptor};
use rusttable_processing::operation_stack::{
    InsertPosition, OperationInstance, OperationStackSnapshot, OperationStackTemplate,
    StackCommand, StackStage,
};

fn source() -> SourceDescriptor {
    SourceDescriptor::new(
        SourceIdentity::new([7; 32]),
        ImageDimensions::new(16, 12).expect("dimensions"),
        Orientation::Normal,
        Roi::new(0, 0, 16, 12).expect("bounds"),
        PixelFormat::rgba8(),
        ColorIdentity::new(ColorEncoding::SrgbD65, 1).expect("source color"),
    )
    .expect("source")
}

fn output(color: ColorIdentity) -> OutputSpec {
    OutputSpec::new(
        ImageDimensions::new(16, 12).expect("dimensions"),
        Roi::new(0, 0, 16, 12).expect("ROI"),
        PixelFormat::rgba8(),
        color,
        Background::transparent(),
    )
    .expect("output")
}

fn implementation() -> ImplementationIdentity {
    ImplementationIdentity::new("rusttable.test", 1, "test-build").expect("implementation")
}

fn stack_with_exposure() -> OperationStackSnapshot {
    let operation = OperationInstance::new(
        11,
        exposure_descriptor().id,
        vec![0, 1],
        StackStage::SceneLinear,
        false,
        true,
    )
    .expect("operation");
    OperationStackSnapshot::new(OperationStackTemplate::raster_basic())
        .apply(StackCommand::Insert {
            operation,
            position: InsertPosition::End,
        })
        .expect("insert")
        .snapshot
}

fn snapshot(stack: OperationStackSnapshot) -> PipelineSnapshot {
    PipelineSnapshot::new(
        PipelineSnapshotInput::new(
            PipelineGeneration::new(1).expect("generation"),
            source(),
            stack,
            output(ColorIdentity::new(ColorEncoding::SrgbD65, 1).expect("output color")),
            PipelinePurpose::Preview,
            implementation(),
        )
        .expect("snapshot input")
        .with_precision(Precision::F32),
    )
    .expect("snapshot")
}

#[test]
fn snapshots_are_immutable_and_diff_one_identity_component() {
    let first = snapshot(OperationStackSnapshot::new(
        OperationStackTemplate::raster_basic(),
    ));
    let second = snapshot(stack_with_exposure());

    assert_ne!(first.identity(), second.identity());
    assert!(first.stack().operations().is_empty());
    assert_eq!(
        first.diff(&second).components(),
        &[rusttable_pixelpipe::SnapshotDiffComponent::Stack]
    );
    assert!(first.publication_is_current(first.publication_generation()));
}

#[test]
fn preparation_calls_the_source_once_per_enabled_operation_and_receipts_are_bounded() {
    let snapshot = snapshot(stack_with_exposure());
    let source = DescriptorPreparationSource::new([exposure_descriptor()])
        .with_implementation(implementation());
    let prepared = PipelinePreparer::new(&source)
        .prepare(&snapshot)
        .expect("prepare");

    assert_eq!(prepared.nodes().len(), 1);
    assert_eq!(prepared.receipt().node_count(), 1);
    assert_eq!(
        prepared.receipt().source_identity(),
        SourceIdentity::new([7; 32])
    );
    assert_eq!(
        prepared.snapshot().raster_status(),
        rusttable_pixelpipe::RasterStatus::Prepared
    );
    assert_eq!(prepared.nodes()[0].operation_id(), 11);
}

#[test]
fn disabled_operations_do_not_get_prepared_but_disabled_mandatory_operations_block() {
    let operation = OperationInstance::new(
        12,
        exposure_descriptor().id,
        vec![0, 1],
        StackStage::SceneLinear,
        false,
        true,
    )
    .expect("operation");
    let stack = OperationStackSnapshot::new(OperationStackTemplate::raster_basic())
        .apply(StackCommand::Insert {
            operation,
            position: InsertPosition::End,
        })
        .expect("insert")
        .snapshot
        .apply(StackCommand::SetEnabled {
            id: 12,
            enabled: false,
        })
        .expect("disable")
        .snapshot;
    let snapshot = snapshot(stack);
    let source = DescriptorPreparationSource::new([exposure_descriptor()])
        .with_implementation(implementation());

    assert!(
        PipelinePreparer::new(&source)
            .prepare(&snapshot)
            .expect("prepare")
            .nodes()
            .is_empty()
    );
}

struct CountingSource {
    calls: Cell<usize>,
    descriptor: OperationDescriptor,
}

impl OperationPreparationSource for CountingSource {
    fn prepare(
        &self,
        _operation: &OperationInstance,
        _context: PreparationContext,
    ) -> Result<PreparedOperation, PreparationSourceError> {
        self.calls.set(self.calls.get() + 1);
        Ok(PreparedOperation::new(
            self.descriptor.clone(),
            implementation(),
            ResourceMetadata::new(1, 1, 1, 1).expect("resource"),
        ))
    }
}

#[test]
fn preparation_is_atomic_for_unknown_operations_and_dangling_references() {
    let unknown = snapshot(stack_with_exposure());
    let source = CountingSource {
        calls: Cell::new(0),
        descriptor: OperationDescriptor {
            id: rusttable_processing::descriptor::DescriptorId::new(
                "other",
                "rusttable.other",
                1,
                1,
                1,
            )
            .expect("descriptor ID"),
            ..exposure_descriptor()
        },
    };
    assert!(matches!(
        PipelinePreparer::new(&source).prepare(&unknown),
        Err(PreparationError::DescriptorMismatch { operation_id: 11 })
    ));
    assert_eq!(source.calls.get(), 1);

    let dangling_operation = OperationInstance::new(
        20,
        exposure_descriptor().id,
        vec![0, 1],
        StackStage::SceneLinear,
        false,
        true,
    )
    .expect("operation")
    .with_mask_blend(Some(99), None);
    let dangling_stack = OperationStackSnapshot::new(OperationStackTemplate::raster_basic())
        .apply(StackCommand::Insert {
            operation: dangling_operation,
            position: InsertPosition::End,
        })
        .expect("insert")
        .snapshot;
    let source = DescriptorPreparationSource::new([exposure_descriptor()])
        .with_implementation(implementation());
    assert!(matches!(
        PipelinePreparer::new(&source).prepare(&snapshot(dangling_stack)),
        Err(PreparationError::DanglingMask {
            operation_id: 20,
            mask_id: 99
        })
    ));
}
