use rusttable_processing::operations::enlargecanvas;
use rusttable_processing::{FiniteF32, LinearRgb, RasterDimensions};

use enlargecanvas::{
    CanvasColor, CanvasFill, ENLARGECANVAS_PARAMETER_BYTES, EnlargeCanvasConfig,
    EnlargeCanvasHistoryParameters, EnlargeCanvasParametersV1, EnlargeCanvasPlan,
    EnlargeCanvasPlanError, decode_history,
};
use rusttable_image::{
    AlphaMode, ByteOrder, ChannelLayout, ColorEncoding, ImageDescriptor, ImageDimensions,
    PixelFormat, SampleType, StorageLayout,
};

fn dimensions(width: u32, height: u32) -> RasterDimensions {
    RasterDimensions::new(width, height).expect("dimensions")
}

fn config(left: f32, right: f32, top: f32, bottom: f32, color: CanvasColor) -> EnlargeCanvasConfig {
    EnlargeCanvasConfig::new(left, right, top, bottom, color).expect("config")
}

fn pixel(value: f32) -> LinearRgb {
    LinearRgb::new(
        FiniteF32::new(value).expect("red"),
        FiniteF32::new(value + 10.0).expect("green"),
        FiniteF32::new(value + 20.0).expect("blue"),
    )
}

#[test]
fn v1_history_is_typed_and_future_history_remains_opaque() {
    let parameters =
        EnlargeCanvasParametersV1::new(config(12.5, 25.0, 50.0, 0.0, CanvasColor::Blue));
    let bytes = parameters.to_bytes();
    assert_eq!(bytes.len(), ENLARGECANVAS_PARAMETER_BYTES);
    assert_eq!(
        EnlargeCanvasParametersV1::from_bytes(&bytes).expect("decode"),
        parameters
    );
    assert!(matches!(
        decode_history(1, &bytes).expect("v1 history"),
        EnlargeCanvasHistoryParameters::V1(value) if value == parameters
    ));
    assert!(matches!(
        decode_history(9, &[1, 2, 3]).expect("opaque history"),
        EnlargeCanvasHistoryParameters::Opaque { version: 9, bytes } if bytes == [1, 2, 3]
    ));
    assert!(EnlargeCanvasParametersV1::from_bytes(&[0; 19]).is_err());
}

#[test]
fn geometry_uses_checked_floor_rounding_and_source_offset() {
    let plan = EnlargeCanvasPlan::new(
        config(50.0, 25.0, 50.0, 50.0, CanvasColor::Black),
        dimensions(4, 3),
    )
    .expect("plan");
    assert_eq!(plan.geometry().left(), 2);
    assert_eq!(plan.geometry().right(), 1);
    assert_eq!(plan.geometry().top(), 1);
    assert_eq!(plan.geometry().bottom(), 1);
    assert_eq!(plan.output_dimensions(), dimensions(7, 5));
    assert_eq!(plan.geometry().source_rect().x(), 2);
    assert_eq!(plan.geometry().source_rect().y(), 1);

    let mut points = [0.5, 1.5, 3.0, 2.0];
    plan.forward_transform(&mut points).expect("forward");
    assert!(
        points
            .iter()
            .zip([2.5, 2.5, 5.0, 3.0])
            .all(|(actual, expected)| { (*actual - expected).abs() < 1.0e-6 })
    );
    plan.back_transform(&mut points).expect("back");
    assert!(
        points
            .iter()
            .zip([0.5, 1.5, 3.0, 2.0])
            .all(|(actual, expected)| { (*actual - expected).abs() < 1.0e-6 })
    );
}

#[test]
fn roi_input_is_only_the_source_intersection() {
    let plan = EnlargeCanvasPlan::new(
        config(100.0, 0.0, 0.0, 0.0, CanvasColor::White),
        dimensions(3, 2),
    )
    .expect("plan");
    let canvas_only = rusttable_image::Roi::new(0, 0, 3, 2).expect("ROI");
    assert_eq!(plan.modify_roi_in(canvas_only).expect("ROI mapping"), None);
    let source_tile = rusttable_image::Roi::new(3, 0, 2, 2).expect("ROI");
    assert_eq!(
        plan.modify_roi_in(source_tile)
            .expect("ROI mapping")
            .expect("source"),
        rusttable_image::Roi::new(0, 0, 2, 2).expect("source ROI")
    );
    assert_eq!(
        plan.modify_roi_out(rusttable_image::Roi::new(0, 0, 3, 2).expect("source ROI"))
            .expect("output ROI"),
        rusttable_image::Roi::new(0, 0, 6, 2).expect("output ROI")
    );
}

#[test]
fn scalar_fill_copy_and_mask_execution_are_deterministic() {
    let plan = EnlargeCanvasPlan::new(
        config(50.0, 50.0, 50.0, 50.0, CanvasColor::Red),
        dimensions(2, 2),
    )
    .expect("plan");
    let input = vec![pixel(1.0), pixel(2.0), pixel(3.0), pixel(4.0)];
    let first = plan.execute(&input).expect("execution");
    let second = plan.execute(&input).expect("execution");
    assert_eq!(first, second);
    assert_eq!(first.dimensions(), dimensions(4, 4));
    assert_eq!(first.pixels()[0], CanvasColor::Red.fill().rgb_pixel());
    assert_eq!(first.pixels()[5], pixel(1.0));
    assert_eq!(first.pixels()[6], pixel(2.0));
    assert_eq!(first.pixels()[9], pixel(3.0));
    assert_eq!(first.pixels()[10], pixel(4.0));
    assert!(matches!(
        plan.execute_with_cancel(&input, || true),
        Err(enlargecanvas::EnlargeCanvasExecutionError::Cancelled)
    ));

    let mask = plan.execute_mask(&[1.0, 2.0, 3.0, 4.0]).expect("mask");
    assert!(mask[0].abs() < 1.0e-6);
    assert!(
        mask[5..7]
            .iter()
            .zip([1.0, 2.0])
            .all(|(actual, expected)| (*actual - expected).abs() < 1.0e-6)
    );
    assert!(
        mask[9..11]
            .iter()
            .zip([3.0, 4.0])
            .all(|(actual, expected)| (*actual - expected).abs() < 1.0e-6)
    );
}

#[test]
fn image_contract_preserves_layout_and_copies_padded_rows() {
    let plan = EnlargeCanvasPlan::new_with_fill(
        config(50.0, 0.0, 100.0, 0.0, CanvasColor::Black),
        dimensions(2, 1),
        CanvasFill::new(0.25, 0.5, 0.75, 0.5).expect("fill"),
    )
    .expect("plan");
    let format = PixelFormat::new(
        SampleType::F32,
        ChannelLayout::Rgba,
        AlphaMode::Straight,
        ByteOrder::Native,
        StorageLayout::Interleaved,
    )
    .expect("format");
    let descriptor = ImageDescriptor::with_strides(
        ImageDimensions::new(2, 1).expect("dimensions"),
        format,
        ColorEncoding::LinearSrgb,
        None,
        rusttable_image::Orientation::Normal,
        &[36],
    )
    .expect("descriptor");
    let mut input = vec![0_u8; descriptor.byte_length()];
    let offset = descriptor.pixel_offset(0, 0).expect("pixel offset");
    input[offset..offset + 16].copy_from_slice(&bytes([1.0, 2.0, 3.0, 1.0]));
    let offset = descriptor.pixel_offset(1, 0).expect("pixel offset");
    input[offset..offset + 16].copy_from_slice(&bytes([4.0, 5.0, 6.0, 1.0]));

    let output = plan
        .execute_image(&descriptor, &input)
        .expect("image execution");
    assert_eq!(output.descriptor().dimensions().width(), 3);
    assert_eq!(output.descriptor().dimensions().height(), 2);
    assert_eq!(output.descriptor().format(), format);
    let source_pixel = output
        .descriptor()
        .pixel_offset(1, 1)
        .expect("source pixel");
    assert_eq!(
        &output.bytes()[source_pixel..source_pixel + 16],
        &input[0..16]
    );
    let fill_pixel = output.descriptor().pixel_offset(0, 0).expect("fill pixel");
    let fill = &output.bytes()[fill_pixel..fill_pixel + 16];
    assert!((f32::from_ne_bytes(fill[0..4].try_into().expect("sample")) - 0.25).abs() < 1.0e-6);
    assert!((f32::from_ne_bytes(fill[12..16].try_into().expect("alpha")) - 0.5).abs() < 1.0e-6);
}

fn bytes(values: [f32; 4]) -> [u8; 16] {
    let mut bytes = [0_u8; 16];
    for (index, value) in values.into_iter().enumerate() {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_ne_bytes());
    }
    bytes
}

#[test]
fn invalid_parameters_are_rejected_before_planning() {
    assert!(EnlargeCanvasConfig::new(-1.0, 0.0, 0.0, 0.0, CanvasColor::Green).is_err());
    assert!(EnlargeCanvasConfig::new(f32::NAN, 0.0, 0.0, 0.0, CanvasColor::Green).is_err());
    assert!(matches!(
        EnlargeCanvasPlan::new(
            config(0.0, 0.0, 0.0, 0.0, CanvasColor::Green),
            dimensions(1, 1)
        )
        .expect("identity")
        .modify_roi_in(rusttable_image::Roi::new(2, 0, 0, 1).expect("ROI")),
        Err(EnlargeCanvasPlanError::RoiOutsideOutput)
    ));
}
