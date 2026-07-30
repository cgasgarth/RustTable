use rusttable_image::{ImageDimensions, Roi};
use rusttable_pixelpipe::{
    HistogramAggregator, HistogramChannel, HistogramChannelModel, HistogramMaskPolicy,
    HistogramNonFinitePolicy, HistogramRange, HistogramRaster, HistogramRequest,
};

fn request(
    model: HistogramChannelModel,
    bins: u32,
    roi: Roi,
    mask_policy: HistogramMaskPolicy,
    nonfinite_policy: HistogramNonFinitePolicy,
) -> HistogramRequest {
    HistogramRequest::new(
        model,
        bins,
        HistogramRange::new(0.0, 1.0).expect("range"),
        roi,
        mask_policy,
        nonfinite_policy,
    )
    .expect("request")
}

#[test]
fn raw_rgb_and_lab_channel_models_have_stable_channel_order() {
    assert_eq!(
        HistogramChannelModel::Raw.channels(),
        &[HistogramChannel::Raw]
    );
    assert_eq!(
        HistogramChannelModel::Rgb.channels(),
        &[
            HistogramChannel::Red,
            HistogramChannel::Green,
            HistogramChannel::Blue
        ]
    );
    assert_eq!(
        HistogramChannelModel::Lab.channels(),
        &[
            HistogramChannel::Lightness,
            HistogramChannel::LabA,
            HistogramChannel::LabB
        ]
    );
}

#[test]
fn serial_aggregation_applies_roi_mask_range_and_edge_binning() {
    let dimensions = ImageDimensions::new(4, 2).expect("dimensions");
    let samples = [
        0.0, 0.1, 0.3, 0.7, // row 0
        0.0, 0.5, 0.8, 0.99, // row 1
    ];
    let mask = [0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0];
    let raster = HistogramRaster::with_mask(
        dimensions,
        HistogramChannelModel::Raw,
        &samples,
        Some(&mask),
    )
    .expect("raster");
    let request = request(
        HistogramChannelModel::Raw,
        4,
        Roi::new(1, 0, 3, 2).expect("ROI"),
        HistogramMaskPolicy::IncludeNonZero,
        HistogramNonFinitePolicy::Skip,
    );

    let result = HistogramAggregator::aggregate(&request, raster).expect("aggregate");
    assert_eq!(result.considered_pixels(), 6);
    assert_eq!(result.accepted_pixels(), 4);
    assert_eq!(result.masked_pixels(), 2);
    assert_eq!(
        result.channel(HistogramChannel::Raw).unwrap().counts(),
        &[1, 1, 1, 1]
    );
}

#[test]
fn tile_merge_is_equivalent_to_serial_aggregation() {
    let dimensions = ImageDimensions::new(5, 3).expect("dimensions");
    let samples: Vec<f32> = (0_u16..15).map(|index| f32::from(index) / 15.0).collect();
    let raster =
        HistogramRaster::new(dimensions, HistogramChannelModel::Raw, &samples).expect("raster");
    let request = request(
        HistogramChannelModel::Raw,
        5,
        Roi::full(dimensions),
        HistogramMaskPolicy::Ignore,
        HistogramNonFinitePolicy::Skip,
    );
    let serial = HistogramAggregator::aggregate(&request, raster).expect("serial");
    let tiled = HistogramAggregator::aggregate_tiles(
        &request,
        raster,
        &[
            Roi::new(0, 0, 2, 2).expect("tile"),
            Roi::new(2, 0, 3, 2).expect("tile"),
            Roi::new(0, 2, 2, 1).expect("tile"),
            Roi::new(2, 2, 3, 1).expect("tile"),
        ],
    )
    .expect("tiles");
    assert_eq!(serial, tiled);
}

#[test]
fn nonfinite_skip_and_reject_policies_are_typed() {
    let dimensions = ImageDimensions::new(3, 1).expect("dimensions");
    let samples = [0.1, f32::NAN, 0.9];
    let raster = HistogramRaster::new(dimensions, HistogramChannelModel::Raw, &samples)
        .expect("raster permits policy-level nonfinite values");
    let skip = request(
        HistogramChannelModel::Raw,
        2,
        Roi::full(dimensions),
        HistogramMaskPolicy::Ignore,
        HistogramNonFinitePolicy::Skip,
    );
    let result = HistogramAggregator::aggregate(&skip, raster).expect("skip");
    assert_eq!(result.accepted_pixels(), 2);
    assert_eq!(result.skipped_nonfinite_pixels(), 1);

    let reject = request(
        HistogramChannelModel::Raw,
        2,
        Roi::full(dimensions),
        HistogramMaskPolicy::Ignore,
        HistogramNonFinitePolicy::Reject,
    );
    assert!(matches!(
        HistogramAggregator::aggregate(&reject, raster),
        Err(rusttable_pixelpipe::HistogramAggregationError::NonFinite {
            pixel_index: 1,
            channel: HistogramChannel::Raw,
        })
    ));
}

#[test]
fn invalid_request_and_merge_contracts_are_rejected() {
    assert!(HistogramRange::new(1.0, 1.0).is_err());
    assert!(
        HistogramRequest::new(
            HistogramChannelModel::Raw,
            0,
            HistogramRange::new(0.0, 1.0).expect("range"),
            Roi::new(0, 0, 1, 1).expect("ROI"),
            HistogramMaskPolicy::Ignore,
            HistogramNonFinitePolicy::Skip,
        )
        .is_err()
    );

    let dimensions = ImageDimensions::new(1, 1).expect("dimensions");
    let raster =
        HistogramRaster::new(dimensions, HistogramChannelModel::Raw, &[0.5]).expect("raster");
    let mut first = HistogramAggregator::aggregate(
        &request(
            HistogramChannelModel::Raw,
            2,
            Roi::full(dimensions),
            HistogramMaskPolicy::Ignore,
            HistogramNonFinitePolicy::Skip,
        ),
        raster,
    )
    .expect("result");
    let second_request = request(
        HistogramChannelModel::Raw,
        3,
        Roi::full(dimensions),
        HistogramMaskPolicy::Ignore,
        HistogramNonFinitePolicy::Skip,
    );
    let second = HistogramAggregator::aggregate(&second_request, raster).expect("result");
    assert_eq!(
        first.merge(&second),
        Err(rusttable_pixelpipe::HistogramMergeError::RequestMismatch)
    );
}
