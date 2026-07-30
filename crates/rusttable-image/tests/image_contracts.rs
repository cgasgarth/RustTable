use rusttable_image::{
    AlignedRgbaF32, AlphaMode, ByteOrder, CfaPattern, ColorEncoding, ImageDescriptor,
    ImageDimensions, ImageView, ImageViewError, Orientation, PixelFormat, Roi, SampleType,
    StorageLayout,
};

#[test]
fn format_descriptor_covers_interleaved_and_planar_storage() {
    let dimensions = ImageDimensions::new(3, 2).expect("dimensions");
    let rgba = PixelFormat::new(
        SampleType::U16,
        rusttable_image::ChannelLayout::Rgba,
        AlphaMode::Straight,
        ByteOrder::Little,
        StorageLayout::Interleaved,
    )
    .expect("RGBA format");
    let descriptor =
        ImageDescriptor::new(dimensions, rgba, ColorEncoding::Srgb, Orientation::Normal)
            .expect("descriptor");
    assert_eq!(descriptor.planes().len(), 1);
    assert_eq!(descriptor.planes()[0].row_stride(), 24);
    assert_eq!(descriptor.byte_length(), 48);

    let planar = PixelFormat::new(
        SampleType::U8,
        rusttable_image::ChannelLayout::Rgb,
        AlphaMode::None,
        ByteOrder::Native,
        StorageLayout::Planar,
    )
    .expect("planar RGB format");
    let descriptor = ImageDescriptor::new(
        dimensions,
        planar,
        ColorEncoding::Unspecified,
        Orientation::Normal,
    )
    .expect("planar descriptor");
    assert_eq!(descriptor.planes().len(), 3);
    assert_eq!(descriptor.byte_length(), 18);
}

#[test]
fn padded_views_and_mutable_rows_are_checked() {
    let dimensions = ImageDimensions::new(2, 2).expect("dimensions");
    let descriptor = ImageDescriptor::with_strides(
        dimensions,
        PixelFormat::rgba8(),
        ColorEncoding::Unspecified,
        None,
        Orientation::Normal,
        &[12],
    )
    .expect("padded descriptor");
    let mut bytes = vec![0_u8; descriptor.byte_length()];
    {
        let mut view = rusttable_image::ImageViewMut::new(&descriptor, &mut bytes).expect("view");
        view.row_mut(0, 1).expect("row")[0] = 9;
    }
    let view = ImageView::new(&descriptor, &bytes).expect("immutable view");
    assert_eq!(view.row(0, 1).expect("row")[0], 9);
    assert_eq!(descriptor.pixel_offset(1, 1), Ok(16));
    assert!(matches!(
        ImageView::new(&descriptor, &[0; 3]),
        Err(ImageViewError::BufferTooShort {
            required: _,
            actual: 3
        })
    ));
}

#[test]
fn all_orientations_swap_dimensions_and_round_trip_coordinates() {
    let source = ImageDimensions::new(3, 2).expect("dimensions");
    for orientation in [
        Orientation::Normal,
        Orientation::FlipHorizontal,
        Orientation::Rotate180,
        Orientation::FlipVertical,
        Orientation::Transpose,
        Orientation::Rotate90,
        Orientation::Transverse,
        Orientation::Rotate270,
    ] {
        let output = orientation.output_dimensions(source);
        for y in 0..source.height() {
            for x in 0..source.width() {
                let mapped = orientation.map_source_to_output(source, x, y);
                let back = orientation
                    .inverse()
                    .map_source_to_output(output, mapped.0, mapped.1);
                assert_eq!((x, y), back, "orientation {orientation:?}");
            }
        }
    }
}

#[test]
fn cfa_phase_tracks_crop_and_orientation() {
    let pattern = CfaPattern::bayer_rggb();
    let phase = rusttable_image::CfaPhase::new(0, 0, pattern);
    let roi = Roi::new(1, 1, 2, 2).expect("ROI");
    assert_eq!(pattern.phase_after_crop(phase, roi).x(), 1);
    assert_eq!(pattern.phase_after_crop(phase, roi).y(), 1);
    assert_eq!(
        pattern.color_at(0, 0, pattern.phase_after_crop(phase, roi)),
        rusttable_image::CfaColor::Blue
    );
    let source = ImageDimensions::new(3, 2).expect("source dimensions");
    let rotated = pattern.phase_after_orientation(phase, source, Orientation::Rotate90);
    assert_eq!((rotated.x(), rotated.y()), (0, 1));
}

#[test]
fn canonical_pixels_are_finite_and_at_least_cacheline_aligned() {
    assert_eq!(std::mem::size_of::<AlignedRgbaF32>(), 16);
    assert_eq!(std::mem::align_of::<AlignedRgbaF32>(), 4);
    let dimensions = ImageDimensions::new(1, 1).expect("dimensions");
    let image = rusttable_image::CanonicalRgbaF32::new(
        dimensions,
        vec![AlignedRgbaF32::new([0.0, 0.5, 1.0, 1.0])],
    )
    .expect("canonical image");
    assert_eq!(image.format(), PixelFormat::canonical_rgba_f32());
    assert_eq!(image.color_encoding(), ColorEncoding::LinearSrgb);
    assert_eq!(
        image.pixel(0).expect("pixel").channels().map(f32::to_bits),
        [0.0_f32, 0.5, 1.0, 1.0].map(f32::to_bits)
    );
    assert_eq!(
        std::ptr::from_ref(image.pixel(0).expect("pixel")).align_offset(16),
        0
    );
    assert!(image.is_cacheline_aligned());
}
