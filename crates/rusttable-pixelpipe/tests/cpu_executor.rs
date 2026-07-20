use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_pixelpipe::{
    CpuImplementation, CpuPixelpipeError, CpuPixelpipeExecutor, CpuPixelpipeOutputMode,
    CpuPixelpipeRequest, RgbaF32Channel, RgbaF32ColorEncoding, RgbaF32Descriptor, RgbaF32Image,
    RgbaF32ImageError, RgbaF32Pixel,
};
use rusttable_processing::{CompiledOperationGraph, RasterDimensions};

fn operation(id: u128, key: &str, parameters: &[(&str, f64)]) -> Operation {
    Operation::new_with_opacity(
        OperationId::new(id).expect("nonzero ID"),
        OperationKey::new(key).expect("valid key"),
        true,
        OperationOpacity::ONE,
        parameters.iter().map(|(name, value)| {
            (
                ParameterName::new(*name).expect("valid parameter"),
                ParameterValue::Scalar(FiniteF64::new(*value).expect("finite value")),
            )
        }),
    )
    .expect("valid operation")
}

fn graph(operations: Vec<Operation>) -> CompiledOperationGraph {
    let edit = Edit::from_parts(
        EditId::new(1).expect("nonzero edit ID"),
        PhotoId::new(2).expect("nonzero photo ID"),
        Revision::ZERO,
        Revision::from_u64(3),
        operations,
    )
    .expect("valid edit");
    CompiledOperationGraph::compile(&edit).expect("registered operations")
}

fn image() -> RgbaF32Image {
    let descriptor = RgbaF32Descriptor::new(
        RasterDimensions::new(2, 1).expect("nonzero dimensions"),
        RgbaF32ColorEncoding::SrgbD65,
    );
    RgbaF32Image::new(
        descriptor,
        vec![
            RgbaF32Pixel::new(0.5, 0.25, 0.75, 0.4),
            RgbaF32Pixel::new(0.1, 0.2, 0.3, 1.0),
        ],
    )
    .expect("valid input")
}

#[test]
fn executes_registered_operations_in_authored_order_and_preserves_alpha() {
    let graph = graph(vec![
        operation(7, "rusttable.exposure", &[("stops", 1.0)]),
        operation(8, "rusttable.linear_offset", &[("value", 0.1)]),
        operation(
            9,
            "rusttable.rgb_gain",
            &[("red", 0.5), ("green", 1.5), ("blue", 2.0)],
        ),
    ]);
    let request = CpuPixelpipeRequest::new(image(), graph, CpuPixelpipeOutputMode::FullExport);

    let result = CpuPixelpipeExecutor
        .execute(&request)
        .expect("CPU execution succeeds");

    assert_eq!(
        result.image().descriptor().color_encoding(),
        RgbaF32ColorEncoding::LinearSrgbD65
    );
    assert!((result.image().pixels()[0].alpha() - 0.4).abs() < f32::EPSILON);
    assert!((result.image().pixels()[1].alpha() - 1.0).abs() < f32::EPSILON);
    assert_eq!(
        result
            .receipt()
            .nodes()
            .iter()
            .map(|node| (node.index(), node.operation_id().get()))
            .collect::<Vec<_>>(),
        [(0, 7), (1, 8), (2, 9)]
    );

    let first = result.image().pixels()[0];
    assert!((first.red() - 0.264_041_13).abs() < 0.000_001);
    assert!((first.green() - 0.302_628_25).abs() < 0.000_001);
    assert!((first.blue() - 2.290_086_3).abs() < 0.000_001);
}

#[test]
fn receipt_is_deterministic_and_records_scalar_cpu_provenance() {
    let graph = graph(vec![operation(
        7,
        "rusttable.linear_offset",
        &[("value", 0.05)],
    )]);
    let request = CpuPixelpipeRequest::new(image(), graph, CpuPixelpipeOutputMode::FullExport);

    let first = CpuPixelpipeExecutor
        .execute(&request)
        .expect("first execution");
    let second = CpuPixelpipeExecutor
        .execute(&request)
        .expect("second execution");

    assert_eq!(
        first.receipt().implementation(),
        CpuImplementation::ScalarReferenceV1
    );
    assert_eq!(first.receipt(), second.receipt());
    assert_ne!(
        first.receipt().input_identity(),
        first.receipt().output_identity()
    );
}

#[test]
fn rejects_a_nonfinite_rgba_component_at_the_descriptor_boundary() {
    let descriptor = RgbaF32Descriptor::new(
        RasterDimensions::new(1, 1).expect("nonzero dimensions"),
        RgbaF32ColorEncoding::SrgbD65,
    );

    assert_eq!(
        RgbaF32Image::new(descriptor, vec![RgbaF32Pixel::new(f32::NAN, 0.0, 0.0, 1.0)]),
        Err(RgbaF32ImageError::NonFiniteComponent {
            pixel_index: 0,
            channel: RgbaF32Channel::Red,
        })
    );
}

#[test]
fn rejects_linear_input_until_a_linear_request_mode_exists() {
    let descriptor = RgbaF32Descriptor::new(
        RasterDimensions::new(1, 1).expect("nonzero dimensions"),
        RgbaF32ColorEncoding::LinearSrgbD65,
    );
    let input = RgbaF32Image::new(descriptor, vec![RgbaF32Pixel::new(1.5, 0.0, 0.0, 1.0)])
        .expect("linear extended range is valid");
    let request =
        CpuPixelpipeRequest::new(input, graph(Vec::new()), CpuPixelpipeOutputMode::FullExport);

    assert_eq!(
        CpuPixelpipeExecutor.execute(&request),
        Err(CpuPixelpipeError::UnsupportedInputEncoding {
            actual: RgbaF32ColorEncoding::LinearSrgbD65,
        })
    );
}

#[test]
fn output_modes_have_known_linear_and_srgb_boundaries_with_identical_alpha() {
    let input = image();
    let graph = graph(Vec::new());
    let full = CpuPixelpipeExecutor
        .execute(&CpuPixelpipeRequest::new(
            input.clone(),
            graph.clone(),
            CpuPixelpipeOutputMode::FullExport,
        ))
        .expect("full export succeeds");
    let preview = CpuPixelpipeExecutor
        .execute(&CpuPixelpipeRequest::new(
            input,
            graph,
            CpuPixelpipeOutputMode::Preview,
        ))
        .expect("preview succeeds");

    assert_eq!(
        full.image().descriptor().color_encoding(),
        RgbaF32ColorEncoding::LinearSrgbD65
    );
    assert_eq!(
        preview.image().descriptor().color_encoding(),
        RgbaF32ColorEncoding::SrgbD65
    );
    let full_pixel = full.image().pixels()[0];
    assert!((full_pixel.red() - 0.214_041_14).abs() < 0.000_001);
    assert!((full_pixel.green() - 0.050_876_09).abs() < 0.000_001);
    assert!((full_pixel.blue() - 0.522_521_56).abs() < 0.000_001);
    let preview_pixel = preview.image().pixels()[0];
    assert!((preview_pixel.red() - 0.5).abs() < 0.000_001);
    assert!((preview_pixel.green() - 0.25).abs() < 0.000_001);
    assert!((preview_pixel.blue() - 0.75).abs() < 0.000_001);
    assert!((full_pixel.alpha() - 0.4).abs() < f32::EPSILON);
    assert!((preview_pixel.alpha() - 0.4).abs() < f32::EPSILON);
    assert_eq!(
        full.receipt().output_mode(),
        CpuPixelpipeOutputMode::FullExport
    );
    assert_eq!(
        preview.receipt().output_mode(),
        CpuPixelpipeOutputMode::Preview
    );
    assert_ne!(
        full.receipt().output_identity(),
        preview.receipt().output_identity()
    );
}
