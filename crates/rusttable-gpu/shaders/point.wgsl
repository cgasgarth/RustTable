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
    output_pixels[id.x] = preserve_alpha(pixel.rgb * multiplier, pixel.a);
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
