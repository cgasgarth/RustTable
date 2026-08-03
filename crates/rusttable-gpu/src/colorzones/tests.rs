#![allow(
    clippy::cast_precision_loss,
    reason = "focused shader tests construct exact source-domain LUT coordinates"
)]
#![expect(
    clippy::suboptimal_flops,
    reason = "GPU reference assertions preserve the native arithmetic operation order."
)]

use super::*;

fn identity() -> ColorZonesRequestIdentity {
    ColorZonesRequestIdentity::new([0x5a; 32])
}

fn device_limits() -> wgpu::Limits {
    let mut limits = wgpu::Limits::default();
    limits.max_bind_groups = limits.max_bind_groups.max(1);
    limits.max_bindings_per_bind_group = limits.max_bindings_per_bind_group.max(BINDING_COUNT);
    limits.max_storage_buffers_per_shader_stage = limits
        .max_storage_buffers_per_shader_stage
        .max(STORAGE_BINDING_COUNT);
    limits.max_uniform_buffers_per_shader_stage =
        limits.max_uniform_buffers_per_shader_stage.max(1);
    limits.max_storage_buffer_binding_size = limits.max_storage_buffer_binding_size.max(LUT_BYTES);
    limits.max_uniform_buffer_binding_size =
        limits.max_uniform_buffer_binding_size.max(PARAMS_SIZE);
    limits.max_buffer_size = limits.max_buffer_size.max(LUT_BYTES);
    limits.max_compute_invocations_per_workgroup = limits
        .max_compute_invocations_per_workgroup
        .max(WORKGROUP_SIZE);
    limits.max_compute_workgroup_size_x = limits.max_compute_workgroup_size_x.max(WORKGROUP_SIZE);
    limits.max_compute_workgroup_size_y = limits.max_compute_workgroup_size_y.max(1);
    limits.max_compute_workgroup_size_z = limits.max_compute_workgroup_size_z.max(1);
    limits.max_compute_workgroups_per_dimension =
        limits.max_compute_workgroups_per_dimension.max(1);
    limits
}

fn request<'a>(
    pixels: &'a [[f32; 4]],
    lightness: &'a [f32],
    chroma: &'a [f32],
    hue: &'a [f32],
    budget: u64,
) -> ColorZonesRequest<'a> {
    ColorZonesRequest::new(
        pixels,
        lightness,
        chroma,
        hue,
        ColorZonesSelection::Hue,
        ColorZonesMode::Smooth,
        identity(),
        budget,
    )
}

#[test]
fn dedicated_colorzones_shader_parses_and_validates() {
    let module =
        naga::front::wgsl::parse_str(COLORZONES_SHADER_SOURCE).expect("Color Zones shader syntax");
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .expect("Color Zones shader validation");
}

#[test]
fn parameter_layout_is_stable_and_keeps_native_tags_separate() {
    let lut = vec![0.5; COLORZONES_LUT_RESOLUTION];
    let pixels = [[50.0, 10.0, -20.0, 0.25]];
    let request = ColorZonesRequest::new(
        &pixels,
        &lut,
        &lut,
        &lut,
        ColorZonesSelection::Chroma,
        ColorZonesMode::Strong,
        identity(),
        u64::MAX,
    );
    let bytes = pack_params(request, 17);
    assert_eq!(
        u32::from_le_bytes(bytes[0..4].try_into().expect("count")),
        17
    );
    assert_eq!(
        u32::from_le_bytes(bytes[4..8].try_into().expect("selection")),
        1
    );
    assert_eq!(
        u32::from_le_bytes(bytes[8..12].try_into().expect("mode")),
        1
    );
    assert_eq!(&bytes[12..16], &[0; 4]);
}

#[test]
fn validation_requires_three_exact_finite_lut_resources() {
    let full = vec![0.5; COLORZONES_LUT_RESOLUTION];
    let short = vec![0.5; COLORZONES_LUT_RESOLUTION - 1];
    let pixels = [[50.0, 0.0, 0.0, 1.0]];
    let limits = device_limits();

    for (actual, expected_resource) in [
        (
            request(&pixels, &short, &full, &full, u64::MAX),
            ColorZonesLutResource::Lightness,
        ),
        (
            request(&pixels, &full, &short, &full, u64::MAX),
            ColorZonesLutResource::Chroma,
        ),
        (
            request(&pixels, &full, &full, &short, u64::MAX),
            ColorZonesLutResource::Hue,
        ),
    ] {
        assert_eq!(
            ValidatedRequest::new(actual, &limits),
            Err(ColorZonesError::InvalidLut {
                resource: expected_resource,
                expected: COLORZONES_LUT_RESOLUTION,
                actual: COLORZONES_LUT_RESOLUTION - 1,
            })
        );
    }

    let mut non_finite = full.clone();
    non_finite[31_337] = f32::NAN;
    assert_eq!(
        ValidatedRequest::new(
            request(&pixels, &full, &non_finite, &full, u64::MAX),
            &limits,
        ),
        Err(ColorZonesError::NonFiniteLut {
            resource: ColorZonesLutResource::Chroma,
            index: 31_337,
        })
    );
}

#[test]
fn validation_rejects_empty_and_non_finite_lab_input() {
    let lut = vec![0.5; COLORZONES_LUT_RESOLUTION];
    let limits = device_limits();
    assert_eq!(
        ValidatedRequest::new(request(&[], &lut, &lut, &lut, u64::MAX), &limits),
        Err(ColorZonesError::EmptyInput)
    );

    let pixels = [[50.0, 1.0, f32::INFINITY, 0.25]];
    assert_eq!(
        ValidatedRequest::new(request(&pixels, &lut, &lut, &lut, u64::MAX), &limits,),
        Err(ColorZonesError::NonFiniteInput {
            pixel: 0,
            component: 2,
        })
    );
}

#[test]
fn validation_bounds_each_buffer_binding_and_aggregate_transient_memory() {
    let lut = vec![0.5; COLORZONES_LUT_RESOLUTION];
    let pixels = [[50.0, 1.0, -2.0, 0.25], [75.0, -3.0, 4.0, 0.75]];
    let pixel_bytes = u64::try_from(pixels.len() * PIXEL_BYTES).expect("pixel bytes");
    let required = aggregate_transient_bytes(pixel_bytes).expect("resource sum");
    assert_eq!(
        colorzones_transient_memory_bytes(pixels.len()),
        Some(required)
    );
    let limits = device_limits();

    let validated = ValidatedRequest::new(request(&pixels, &lut, &lut, &lut, required), &limits)
        .expect("exact memory budget");
    assert_eq!(validated.aggregate_transient_bytes, required);
    assert_eq!(validated.workgroups, 1);
    assert_eq!(
        ValidatedRequest::new(request(&pixels, &lut, &lut, &lut, required - 1), &limits,),
        Err(ColorZonesError::AggregateMemoryLimit {
            required,
            limit: required - 1,
        })
    );

    let mut too_small = limits;
    too_small.max_storage_buffer_binding_size = LUT_BYTES - 1;
    assert_eq!(
        ValidatedRequest::new(request(&pixels, &lut, &lut, &lut, u64::MAX), &too_small,),
        Err(ColorZonesError::BufferLimit {
            resource: "LUT buffer",
            required: LUT_BYTES,
            limit: LUT_BYTES - 1,
        })
    );
}

#[test]
fn in_place_dispatch_fits_downlevel_bindings_and_rejects_one_fewer() {
    let lut = vec![0.5; COLORZONES_LUT_RESOLUTION];
    let pixels = [[50.0, 0.0, 0.0, 1.0]];
    let downlevel = wgpu::Limits::downlevel_defaults();
    ValidatedRequest::new(request(&pixels, &lut, &lut, &lut, u64::MAX), &downlevel)
        .expect("one pixel buffer plus three immutable LUTs fit downlevel WGPU");

    let mut limits = device_limits();
    limits.max_storage_buffers_per_shader_stage = STORAGE_BINDING_COUNT - 1;
    assert_eq!(
        ValidatedRequest::new(request(&pixels, &lut, &lut, &lut, u64::MAX), &limits,),
        Err(ColorZonesError::InsufficientComputeLimits)
    );
}

#[test]
fn cancellation_attachment_preserves_request_identity() {
    let lut = vec![0.5; COLORZONES_LUT_RESOLUTION];
    let pixels = [[50.0, 0.0, 0.0, f32::from_bits(1)]];
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let request = request(&pixels, &lut, &lut, &lut, u64::MAX).with_cancellation(&cancellation);
    assert!(request.is_cancelled());
    assert_eq!(request.identity(), identity());
    assert_eq!(request.identity().as_bytes(), [0x5a; 32]);
}

#[tokio::test]
async fn gpu_uses_native_nearest_lookup_and_clamps_both_endpoints_when_available() {
    let Ok(runtime) = GpuRuntime::initialize(crate::GpuRuntimeConfig::default()).await else {
        return;
    };
    if runtime.is_cpu_only() {
        return;
    }

    let mut lightness = vec![0.5; COLORZONES_LUT_RESOLUTION];
    let chroma = vec![0.5; COLORZONES_LUT_RESOLUTION];
    let hue = vec![0.5; COLORZONES_LUT_RESOLUTION];
    let selected_index = 12_345_usize;
    lightness[selected_index] = 0.75;
    lightness[selected_index + 1] = 0.25;
    lightness[0] = 0.75;
    lightness[COLORZONES_LUT_RESOLUTION - 1] = 0.25;
    let selection = (selected_index as f32 + 0.25) / COLORZONES_LUT_RESOLUTION as f32;
    let pixels = [
        [selection * 100.0, 8.0, 0.0, f32::from_bits(1)],
        [-10.0, 8.0, 0.0, f32::from_bits(2)],
        [150.0, 8.0, 0.0, f32::from_bits(3)],
    ];
    let budget =
        aggregate_transient_bytes(u64::try_from(pixels.len() * PIXEL_BYTES).expect("pixel bytes"))
            .expect("resource sum");
    let result = runtime
        .execute_colorzones(ColorZonesRequest::new(
            &pixels,
            &lightness,
            &chroma,
            &hue,
            ColorZonesSelection::Lightness,
            ColorZonesMode::Strong,
            identity(),
            budget,
        ))
        .expect("Color Zones Strong dispatch");

    let expected_lightness = [pixels[0][0] * 2.0, -20.0, 75.0];
    let cpu_interpolated_lut = 0.75 * 0.75 + 0.25 * 0.25;
    let cpu_interpolated_lightness =
        pixels[0][0] * 2.0_f32.powf(4.0 * (cpu_interpolated_lut - 0.5));
    assert!(
        (result.pixels()[0][0] - cpu_interpolated_lightness).abs() > 1.0,
        "the source OpenCL nearest lookup must remain observably distinct from CPU interpolation"
    );
    for (index, (actual, expected)) in result.pixels().iter().zip(expected_lightness).enumerate() {
        assert!(
            (actual[0] - expected).abs() <= 0.000_02,
            "pixel {index} lightness: {} != {expected}",
            actual[0]
        );
        assert_eq!(actual[3].to_bits(), pixels[index][3].to_bits());
    }
    assert_eq!(result.identity(), identity());
    assert_eq!(result.dispatches(), 1);
}

#[tokio::test]
async fn gpu_keeps_smooth_and_strong_chroma_selection_math_distinct_when_available() {
    let Ok(runtime) = GpuRuntime::initialize(crate::GpuRuntimeConfig::default()).await else {
        return;
    };
    if runtime.is_cpu_only() {
        return;
    }

    let lightness = vec![0.5; COLORZONES_LUT_RESOLUTION];
    let mut chroma = vec![0.5; COLORZONES_LUT_RESOLUTION];
    chroma[30_000..].fill(1.0);
    let hue = vec![0.5; COLORZONES_LUT_RESOLUTION];
    let pixels = [[50.0, 64.0, 0.0, f32::from_bits(0x3eaa_aaab)]];
    let budget =
        aggregate_transient_bytes(u64::try_from(PIXEL_BYTES).expect("pixel byte width fits u64"))
            .expect("resource sum");

    let execute = |mode| {
        runtime.execute_colorzones(ColorZonesRequest::new(
            &pixels,
            &lightness,
            &chroma,
            &hue,
            ColorZonesSelection::Chroma,
            mode,
            identity(),
            budget,
        ))
    };
    let smooth = execute(ColorZonesMode::Smooth).expect("Smooth dispatch");
    let strong = execute(ColorZonesMode::Strong).expect("Strong dispatch");

    assert!((smooth.pixels()[0][1] - 128.0).abs() <= 0.000_03);
    assert!((strong.pixels()[0][1] - 64.0).abs() <= 0.000_03);
    assert_eq!(smooth.pixels()[0][3].to_bits(), pixels[0][3].to_bits());
    assert_eq!(strong.pixels()[0][3].to_bits(), pixels[0][3].to_bits());
}
