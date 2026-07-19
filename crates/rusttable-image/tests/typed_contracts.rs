use std::mem::align_of;

use rusttable_image::{
    AlphaMode, BayerPattern, BlackWhiteLevels, BufferPool, ByteOrder, CanonicalRgbaBuffer,
    CfaColor, CfaDescriptor, CfaPhase, ChannelLayout, ColorEncodingReference, Coordinate,
    DefaultBufferPool, ExifOrientation, ImageDimensions, ImageRect, OwnedPlane, PixelFormat,
    PixelFormatError, PlaneDescriptor, PlaneError, RawMosaic, SampleType, StorageLayout,
};

#[test]
fn every_orientation_round_trips_source_coordinates() {
    let dimensions = ImageDimensions::new(7, 5).expect("dimensions");
    for orientation in ExifOrientation::ALL {
        let output = orientation.output_dimensions(dimensions);
        for y in 0..dimensions.height() {
            for x in 0..dimensions.width() {
                let coordinate = Coordinate::new(x, y);
                let transformed = orientation
                    .source_to_output(dimensions, coordinate)
                    .expect("source coordinate");
                assert!(transformed.x() < output.width());
                assert!(transformed.y() < output.height());
                assert_eq!(
                    orientation
                        .output_to_source(dimensions, transformed)
                        .expect("output coordinate"),
                    coordinate
                );
            }
        }
    }
}

#[test]
fn dimensions_and_rois_reject_zero_and_overflowing_ranges() {
    let dimensions = ImageDimensions::new(8, 6).expect("dimensions");
    assert!(ImageRect::new(0, 0, 0, 1, dimensions).is_err());
    assert!(ImageRect::new(u32::MAX, 0, 2, 1, dimensions).is_err());
    assert!(ImageRect::new(7, 5, 2, 1, dimensions).is_err());
    assert!(ImageDimensions::new(u32::MAX, u32::MAX).is_ok());
    assert!(
        ImageDimensions::new(u32::MAX, u32::MAX)
            .expect("dimensions")
            .decoded_byte_count()
            .is_err()
    );
}

#[test]
fn plane_descriptors_reject_full_allocation_overflow() {
    let dimensions = ImageDimensions::new(u32::MAX, u32::MAX).expect("dimensions");
    let format = PixelFormat::new(
        SampleType::U8,
        ChannelLayout::Gray,
        AlphaMode::None,
        ByteOrder::Native,
    )
    .expect("format");
    assert!(
        PlaneDescriptor::new(dimensions, format, StorageLayout::Interleaved, usize::MAX).is_err()
    );
}

#[test]
fn pixel_descriptors_reject_invalid_alpha_and_planar_channels() {
    let format = PixelFormat::new(
        SampleType::U16,
        ChannelLayout::Rgba,
        AlphaMode::Straight,
        ByteOrder::Little,
    )
    .expect("RGBA format");
    assert_eq!(format.bytes_per_pixel(), Ok(8));
    assert_eq!(
        PixelFormat::new(
            SampleType::U8,
            ChannelLayout::Rgb,
            AlphaMode::Straight,
            ByteOrder::Native,
        ),
        Err(PixelFormatError::AlphaNotAllowed {
            layout: ChannelLayout::Rgb,
            alpha: AlphaMode::Straight,
        })
    );
    assert_eq!(
        format.validate_storage(StorageLayout::Planar { plane: 4 }),
        Err(PixelFormatError::InvalidPlane {
            plane: 4,
            channels: 4,
        })
    );
}

#[test]
fn padded_interleaved_and_planar_views_have_checked_offsets() {
    let dimensions = ImageDimensions::new(3, 2).expect("dimensions");
    let format = PixelFormat::new(
        SampleType::U8,
        ChannelLayout::Rgba,
        AlphaMode::Straight,
        ByteOrder::Native,
    )
    .expect("format");
    let descriptor = PlaneDescriptor::new(dimensions, format, StorageLayout::Interleaved, 16)
        .expect("padded descriptor");
    let data: Vec<u8> = (0..32).collect();
    let plane = OwnedPlane::new(descriptor, data).expect("plane");
    assert_eq!(
        plane.view().row(1).expect("row"),
        &[16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27]
    );
    assert_eq!(plane.view().sample_offset(2, 1, 3), Ok(27));
    let crop = ImageRect::new(1, 0, 2, 2, dimensions).expect("crop");
    let cropped = plane.view().crop(crop).expect("view");
    assert_eq!(cropped.sample_offset(0, 0, 0), Ok(0));
    assert!(matches!(
        cropped.sample_offset(2, 0, 0),
        Err(PlaneError::OutOfBounds)
    ));

    let planar_format = PixelFormat::new(
        SampleType::U16,
        ChannelLayout::Rgb,
        AlphaMode::None,
        ByteOrder::Little,
    )
    .expect("planar format");
    let planar = PlaneDescriptor::new(
        dimensions,
        planar_format,
        StorageLayout::Planar { plane: 2 },
        8,
    )
    .expect("planar descriptor");
    assert_eq!(planar.row_bytes(), Ok(6));
    assert_eq!(planar.buffer_bytes(), Ok(16));
}

#[test]
fn cfa_phase_follows_crop_and_orientation_without_copying_samples() {
    let dimensions = ImageDimensions::new(4, 4).expect("dimensions");
    let descriptor = CfaDescriptor::bayer(BayerPattern::Rggb, CfaPhase::new(0, 0));
    assert_eq!(descriptor.color_at(0, 0), CfaColor::Red);
    let crop = ImageRect::new(1, 1, 2, 2, dimensions).expect("crop");
    let cropped = descriptor.crop(crop).expect("cropped CFA");
    assert_eq!(cropped.phase(), CfaPhase::new(1, 1));
    let oriented = cropped.oriented(ExifOrientation::Rotate90);
    let output_coordinate = Coordinate::new(1, 0);
    let source_coordinate = ExifOrientation::Rotate90
        .output_to_source(crop.dimensions(), output_coordinate)
        .expect("source");
    assert_eq!(
        oriented
            .color_at(crop.dimensions(), output_coordinate)
            .expect("color"),
        cropped.color_at(source_coordinate.x(), source_coordinate.y())
    );
}

#[test]
fn canonical_buffer_is_checked_and_aligned() {
    let dimensions = ImageDimensions::new(2, 1).expect("dimensions");
    let pool = DefaultBufferPool;
    let mut buffer = pool.allocate_canonical(dimensions).expect("buffer");
    assert_eq!(buffer.byte_size(), Ok(32));
    assert!(buffer.allocated_byte_size().expect("size") >= buffer.byte_size().expect("size"));
    assert_eq!(CanonicalRgbaBuffer::pixel_alignment(), 16);
    assert!(CanonicalRgbaBuffer::allocation_alignment() >= 64);
    assert_eq!(align_of::<[f32; 4]>(), 4);
    assert_eq!(buffer.pixels().len(), 2);
    assert!(buffer.set_pixel(1, 0, [1.0, 0.5, 0.25, 1.0]));
    assert_eq!(buffer.pixel(1, 0), Some(&[1.0, 0.5, 0.25, 1.0]));
    assert!(!buffer.set_pixel(2, 0, [0.0; 4]));
}

#[test]
fn raw_mosaic_requires_u16_bayer_samples_and_checked_levels() {
    assert!(BlackWhiteLevels::new(0.0, 16383.0).is_ok());
    assert!(BlackWhiteLevels::new(1.0, 1.0).is_err());
    assert!(BlackWhiteLevels::new(f32::NAN, 1.0).is_err());
    assert_eq!(
        ColorEncodingReference::LinearSrgb,
        ColorEncodingReference::LinearSrgb
    );

    let dimensions = ImageDimensions::new(2, 2).expect("dimensions");
    let format = PixelFormat::new(
        SampleType::U16,
        ChannelLayout::Bayer,
        AlphaMode::None,
        ByteOrder::Little,
    )
    .expect("RAW format");
    let descriptor = PlaneDescriptor::new(dimensions, format, StorageLayout::Interleaved, 4)
        .expect("RAW descriptor");
    let plane = OwnedPlane::new(descriptor, vec![0; 8]).expect("RAW plane");
    let cfa = CfaDescriptor::bayer(BayerPattern::Rggb, CfaPhase::new(0, 0));
    let levels = BlackWhiteLevels::new(64.0, 16383.0).expect("levels");
    let mosaic = RawMosaic::new(plane, cfa, levels).expect("mosaic");
    assert_eq!(mosaic.cfa(), cfa);
    assert!((mosaic.levels().white() - 16383.0).abs() <= f32::EPSILON);
}
