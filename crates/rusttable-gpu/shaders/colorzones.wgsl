// Direct WGPU port of data/kernels/basic.cl::colorzones_v3 and colorzones.

const TAU: f32 = 6.28318530717958647692;
const LUT_RESOLUTION: u32 = 65536u;
const LUT_LAST_INDEX: u32 = LUT_RESOLUTION - 1u;

struct ColorZonesParams {
    pixel_count: u32,
    selection_channel: u32,
    mode: u32,
    _padding: u32,
}

struct PixelBuffer {
    values: array<vec4<f32>>,
}

struct LutBuffer {
    values: array<f32>,
}

// Color Zones is point-local, so one read-write pixel buffer is race-free and
// keeps the complete shader within WGPU's downlevel four-storage-buffer limit.
@group(0) @binding(0) var<storage, read_write> pixels: PixelBuffer;
@group(0) @binding(1) var<uniform> params: ColorZonesParams;
@group(0) @binding(2) var<storage, read> lightness_lut: LutBuffer;
@group(0) @binding(3) var<storage, read> chroma_lut: LutBuffer;
@group(0) @binding(4) var<storage, read> hue_lut: LutBuffer;

// data/kernels/color_conversion.h::lookup uses integer image coordinates:
// clamp((int)(x * 0x10000), 0, 0xffff). Clamp before conversion here so the
// safe WGSL boundary remains defined even for extreme finite negative Lab L.
fn nearest_lut_index(selection: f32) -> u32 {
    let bounded = clamp(selection, 0.0, 1.0);
    if (bounded >= 1.0) {
        return LUT_LAST_INDEX;
    }
    return u32(bounded * f32(LUT_RESOLUTION));
}

// Default, non-fast data/kernels/common.h::dt_fast_hypot maps to OpenCL
// hypot(). The scaled form avoids a naive square overflowing before the true
// norm, while the explicit predicate preserves a true f32 overflow on Metal.
fn opencl_default_hypot_overflows(x: f32, y: f32) -> bool {
    let overflow_scale = bitcast<f32>(0x1f000000u);
    let scaled_x = abs(x) * overflow_scale;
    let scaled_y = abs(y) * overflow_scale;
    let maximum_finite = bitcast<f32>(0x7f7fffffu);
    let scaled_maximum = maximum_finite * overflow_scale;
    return scaled_x * scaled_x + scaled_y * scaled_y
        > scaled_maximum * scaled_maximum;
}

fn opencl_default_hypot(x: f32, y: f32) -> f32 {
    let absolute_x = abs(x);
    let absolute_y = abs(y);
    let maximum = max(absolute_x, absolute_y);
    if (maximum == 0.0) {
        return 0.0;
    }
    let ratio = min(absolute_x, absolute_y) / maximum;
    if (opencl_default_hypot_overflows(x, y)) {
        return bitcast<f32>(0x7f800000u);
    }
    return maximum * sqrt(1.0 + ratio * ratio);
}

fn smooth_hue(a: f32, b: f32) -> f32 {
    let positive_angle = atan2(b, a) + TAU;
    return (positive_angle - floor(positive_angle / TAU) * TAU) / TAU;
}

fn strong_hue(a: f32, b: f32) -> f32 {
    let angle = atan2(b, a);
    if (angle > 0.0) {
        return angle / TAU;
    }
    return 1.0 - abs(angle) / TAU;
}

// Direct scalar port of basic.cl::colorzones_v3 (native Smooth mode).
fn process_smooth(pixel: vec4<f32>) -> vec4<f32> {
    let a = pixel.y;
    let b = pixel.z;
    let hue = smooth_hue(a, b);
    let chroma = opencl_default_hypot(b, a);

    var selection = 0.0;
    var blend = 0.0;
    switch params.selection_channel {
        case 0u: {
            selection = min(1.0, pixel.x / 100.0);
        }
        case 1u: {
            selection = min(1.0, chroma / 128.0);
        }
        default: {
            selection = hue;
            let inverse_chroma = 1.0 - chroma / 128.0;
            blend = inverse_chroma * inverse_chroma;
        }
    }

    let lut_index = nearest_lut_index(selection);
    let lightness_modification =
        (blend * 0.5 + (1.0 - blend) * lightness_lut.values[lut_index]) - 0.5;
    let hue_modification =
        (blend * 0.5 + (1.0 - blend) * hue_lut.values[lut_index]) - 0.5;
    // Native intentionally does not apply the low-chroma blend to saturation.
    let chroma_modification = 2.0 * chroma_lut.values[lut_index];
    let lightness = pixel.x * pow(2.0, 4.0 * lightness_modification);
    let adjusted_hue = TAU * (hue + hue_modification);

    return vec4<f32>(
        lightness,
        cos(adjusted_hue) * chroma_modification * chroma,
        sin(adjusted_hue) * chroma_modification * chroma,
        pixel.w,
    );
}

// Direct scalar port of basic.cl::colorzones (native Strong mode). Keep this
// conversion and its selection normalization separate from Smooth mode.
fn process_strong(pixel: vec4<f32>) -> vec4<f32> {
    var lightness = pixel.x;
    var chroma = opencl_default_hypot(pixel.y, pixel.z);
    var hue = strong_hue(pixel.y, pixel.z);
    let normalize_chroma = 1.0 / (128.0 * sqrt(2.0));

    var selection = 0.0;
    switch params.selection_channel {
        case 0u: {
            selection = lightness * 0.01;
        }
        case 1u: {
            selection = chroma * normalize_chroma;
        }
        default: {
            selection = hue;
        }
    }
    selection = clamp(selection, 0.0, 1.0);

    let lut_index = nearest_lut_index(selection);
    lightness = lightness
        * pow(2.0, 4.0 * (lightness_lut.values[lut_index] - 0.5));
    chroma = chroma * (2.0 * chroma_lut.values[lut_index]);
    hue = hue + hue_lut.values[lut_index] - 0.5;
    let adjusted_hue = TAU * hue;

    return vec4<f32>(
        lightness,
        cos(adjusted_hue) * chroma,
        sin(adjusted_hue) * chroma,
        pixel.w,
    );
}

@compute @workgroup_size(256, 1, 1)
fn colorzones(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= params.pixel_count) {
        return;
    }
    let pixel = pixels.values[id.x];
    if (params.mode == 0u) {
        pixels.values[id.x] = process_smooth(pixel);
    } else {
        pixels.values[id.x] = process_strong(pixel);
    }
}
