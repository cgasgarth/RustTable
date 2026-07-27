// Safe WGPU port of Darktable's data/kernels/bloom.cl.

struct BloomParams {
    width: u32,
    height: u32,
    radius: u32,
    _geometry_padding: u32,
    scale: f32,
    threshold: f32,
    _parameter_padding: vec2<u32>,
};

@group(0) @binding(0)
var<storage, read> input_pixels: array<vec4<f32>>;

@group(0) @binding(1)
var<storage, read> light_input: array<f32>;

@group(0) @binding(2)
var<storage, read_write> light_output: array<f32>;

@group(0) @binding(3)
var<storage, read_write> output_pixels: array<vec4<f32>>;

@group(0) @binding(4)
var<uniform> params: BloomParams;

fn pixel_index(x: u32, y: u32) -> u32 {
    return y * params.width + x;
}

@compute @workgroup_size(16, 16, 1)
fn bloom_threshold(@builtin(global_invocation_id) global_id: vec3<u32>) {
    if global_id.x >= params.width || global_id.y >= params.height {
        return;
    }

    let index = pixel_index(global_id.x, global_id.y);
    let lightness = input_pixels[index].x * params.scale;
    light_output[index] = select(0.0, lightness, lightness > params.threshold);
}

@compute @workgroup_size(256, 1, 1)
fn bloom_hblur(@builtin(global_invocation_id) global_id: vec3<u32>) {
    if global_id.x >= params.width || global_id.y >= params.height {
        return;
    }

    let radius = i32(params.radius);
    let maximum_x = i32(params.width) - 1;
    var sum = 0.0;
    var offset = -radius;
    loop {
        if offset > radius {
            break;
        }
        let sample_x = u32(clamp(i32(global_id.x) + offset, 0, maximum_x));
        sum += light_input[pixel_index(sample_x, global_id.y)];
        offset += 1;
    }
    light_output[pixel_index(global_id.x, global_id.y)] =
        sum / f32(2u * params.radius + 1u);
}

@compute @workgroup_size(1, 256, 1)
fn bloom_vblur(@builtin(global_invocation_id) global_id: vec3<u32>) {
    if global_id.x >= params.width || global_id.y >= params.height {
        return;
    }

    let radius = i32(params.radius);
    let maximum_y = i32(params.height) - 1;
    var sum = 0.0;
    var offset = -radius;
    loop {
        if offset > radius {
            break;
        }
        let sample_y = u32(clamp(i32(global_id.y) + offset, 0, maximum_y));
        sum += light_input[pixel_index(global_id.x, sample_y)];
        offset += 1;
    }
    light_output[pixel_index(global_id.x, global_id.y)] =
        sum / f32(2u * params.radius + 1u);
}

@compute @workgroup_size(16, 16, 1)
fn bloom_mix(@builtin(global_invocation_id) global_id: vec3<u32>) {
    if global_id.x >= params.width || global_id.y >= params.height {
        return;
    }

    let index = pixel_index(global_id.x, global_id.y);
    var pixel = input_pixels[index];
    let processed = light_input[index];
    pixel.x = 100.0 - (((100.0 - pixel.x) * (100.0 - processed)) / 100.0);
    output_pixels[index] = pixel;
}
