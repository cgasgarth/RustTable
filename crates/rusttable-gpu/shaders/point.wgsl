// rusttable:include shaders/includes/point_common.wgsl

@compute @workgroup_size(${WORKGROUP_SIZE}, 1, 1)
fn transfer_decode(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!in_bounds(id.x)) { return; }
    let pixel = input_pixels[id.x];
    let decoded = pow(max(pixel.rgb, vec3<f32>(0.0)), vec3<f32>(params.transfer_gamma));
    output_pixels[id.x] = preserve_alpha(decoded, pixel.a);
}

@compute @workgroup_size(${WORKGROUP_SIZE}, 1, 1)
fn transfer_encode(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!in_bounds(id.x)) { return; }
    let pixel = input_pixels[id.x];
    let encoded = pow(max(pixel.rgb, vec3<f32>(0.0)), vec3<f32>(1.0 / params.transfer_gamma));
    output_pixels[id.x] = preserve_alpha(encoded, pixel.a);
}

@compute @workgroup_size(${WORKGROUP_SIZE}, 1, 1)
fn exposure(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!in_bounds(id.x)) { return; }
    let pixel = input_pixels[id.x];
    let multiplier = exp2(params.exposure_stops);
    let white = 1.0 / multiplier;
    let scale = 1.0 / (white - params.black_level);
    output_pixels[id.x] = preserve_alpha((pixel.rgb - vec3<f32>(params.black_level)) * scale, pixel.a);
}

fn basicadj_norm(rgb: vec3<f32>, mode: u32) -> f32 {
    switch mode {
        case 0u: { return (rgb.x + rgb.y + rgb.z) / 3.0; }
        case 1u: { return dot(rgb, vec3<f32>(0.2225045, 0.7168786, 0.0606169)); }
        case 2u: { return max(rgb.x, max(rgb.y, rgb.z)); }
        case 3u: { return (rgb.x + rgb.y + rgb.z) / 3.0; }
        case 4u: { return rgb.x + rgb.y + rgb.z; }
        case 5u: { return sqrt(dot(rgb, rgb)); }
        default: {
            let squares = rgb * rgb;
            let denominator = dot(squares, vec3<f32>(1.0));
            if (denominator == 0.0) { return 0.0; }
            return dot(rgb, squares) / denominator;
        }
    }
}

fn basicadj_hlcurve(level: f32, hlcomp: f32, hlrange: f32) -> f32 {
    if (hlcomp <= 0.0) { return 1.0; }
    var value = level + (hlrange - 1.0);
    if (value == 0.0) { value = 0.000001; }
    var y = value / hlrange * hlcomp;
    if (y <= -1.0) { y = -0.999999; }
    return log(1.0 + y) * (hlrange / (value * hlcomp));
}

@compute @workgroup_size(${WORKGROUP_SIZE}, 1, 1)
fn basicadj(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!in_bounds(id.x)) { return; }
    let pixel = input_pixels[id.x];
    var rgb = (pixel.rgb - vec3<f32>(basic_params.black_point)) * basic_params.scale;
    if (basic_params.hlcomp > 0.0) {
        let luminance = basicadj_norm(rgb, 1u);
        if (luminance > 0.0) {
            rgb = rgb * basicadj_hlcurve(luminance, basic_params.hlcomp, basic_params.hlrange);
        }
    }
    if (basic_params.gamma != 1.0) {
        for (var channel = 0u; channel < 3u; channel++) {
            if (rgb[channel] > 0.0) { rgb[channel] = pow(rgb[channel], basic_params.gamma); }
        }
    }
    if (basic_params.contrast != 1.0) {
        if (basic_params.preserve_colors == 0u) {
            for (var channel = 0u; channel < 3u; channel++) {
                if (rgb[channel] > 0.0) {
                    rgb[channel] = pow(rgb[channel] / basic_params.middle_grey, basic_params.contrast)
                        * basic_params.middle_grey;
                }
            }
        } else {
            let luminance = basicadj_norm(rgb, basic_params.preserve_colors);
            if (luminance > 0.0) {
                let contrasted = pow(luminance / basic_params.middle_grey, basic_params.contrast)
                    * basic_params.middle_grey;
                rgb = rgb * (contrasted / luminance);
            }
        }
    }
    if (basic_params.saturation != 0.0 || basic_params.vibrance != 0.0) {
        let average = (rgb.x + rgb.y + rgb.z) / 3.0;
        let delta = length(rgb - vec3<f32>(average));
        let vibrance = basic_params.vibrance / 1.4;
        let boost = vibrance * (1.0 - pow(delta, abs(vibrance)));
        let factor = basic_params.saturation + 1.0 + boost;
        rgb = vec3<f32>(average) + factor * (rgb - vec3<f32>(average));
    }
    output_pixels[id.x] = preserve_alpha(rgb, pixel.a);
}

@compute @workgroup_size(${WORKGROUP_SIZE}, 1, 1)
fn linear_offset(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!in_bounds(id.x)) { return; }
    let pixel = input_pixels[id.x];
    output_pixels[id.x] = preserve_alpha(pixel.rgb + vec3<f32>(params.linear_offset), pixel.a);
}

@compute @workgroup_size(${WORKGROUP_SIZE}, 1, 1)
fn rgb_gain(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!in_bounds(id.x)) { return; }
    let pixel = input_pixels[id.x];
    let gain = vec3<f32>(params.gain_red, params.gain_green, params.gain_blue);
    output_pixels[id.x] = preserve_alpha(pixel.rgb * gain, pixel.a);
}

fn comparison_clamps(value: f32, low: f32, high: f32) -> f32 {
    if (value > low) {
        if (value < high) {
            return value;
        }
        return high;
    }
    return low;
}

// Direct scalar port of data/kernels/extended.cl::velvia. Keep the authored
// association and comparison clamp: finite inputs can still produce NaN in
// intermediate overflow, which Darktable's CLAMPS routes to the lower bound.
@compute @workgroup_size(${WORKGROUP_SIZE}, 1, 1)
fn velvia(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!in_bounds(id.x)) { return; }
    let pixel = input_pixels[id.x];
    if (params.velvia_strength <= 0.0) {
        output_pixels[id.x] = pixel;
        return;
    }

    let pmax = max(pixel.r, max(pixel.g, pixel.b));
    let pmin = min(pixel.r, min(pixel.g, pixel.b));
    let plum = (pmax + pmin) / 2.0;
    var psat: f32;
    if (plum <= 0.5) {
        psat = (pmax - pmin) / ((0.00001 + pmax) + pmin);
    } else {
        psat = (pmax - pmin) / (0.00001 + max(0.0, (2.0 - pmax) - pmin));
    }
    let pweight = comparison_clamps(
        ((1.0 - (1.5 * psat))
            + ((1.0 + (abs(plum - 0.5) * 2.0)) * (1.0 - params.velvia_bias)))
            / (1.0 + (1.0 - params.velvia_bias)),
        0.0,
        1.0,
    );
    let saturation = params.velvia_strength * pweight;
    let red = comparison_clamps(
        pixel.r + saturation * (pixel.r - 0.5 * (pixel.g + pixel.b)),
        0.0,
        1.0,
    );
    let green = comparison_clamps(
        pixel.g + saturation * (pixel.g - 0.5 * (pixel.b + pixel.r)),
        0.0,
        1.0,
    );
    let blue = comparison_clamps(
        pixel.b + saturation * (pixel.b - 0.5 * (pixel.r + pixel.g)),
        0.0,
        1.0,
    );
    output_pixels[id.x] = vec4<f32>(red, green, blue, pixel.a);
}

@compute @workgroup_size(${WORKGROUP_SIZE}, 1, 1)
fn copy(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!in_bounds(id.x)) { return; }
    output_pixels[id.x] = input_pixels[id.x];
}

@compute @workgroup_size(${WORKGROUP_SIZE}, 1, 1)
fn probe(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!in_bounds(id.x)) { return; }
    output_pixels[id.x] = input_pixels[id.x];
}
