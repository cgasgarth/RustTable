struct PointParams {
    pixel_count: u32,
    _reserved0: vec3<u32>,
    exposure_stops: f32,
    linear_offset: f32,
    gain_red: f32,
    gain_green: f32,
    gain_blue: f32,
    transfer_gamma: f32,
    black_level: f32,
    _reserved1: f32,
}

@group(0) @binding(0) var<storage, read> input_pixels: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> output_pixels: array<vec4<f32>>;
@group(0) @binding(2) var<uniform> params: PointParams;

fn in_bounds(index: u32) -> bool {
    return index < params.pixel_count;
}

fn preserve_alpha(rgb: vec3<f32>, alpha: f32) -> vec4<f32> {
    return vec4<f32>(rgb, alpha);
}
