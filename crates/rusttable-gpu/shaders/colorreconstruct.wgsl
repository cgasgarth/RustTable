// Safe WGPU port of data/kernels/colorreconstruction.cl at Darktable
// baseline d8628e8103989bc4ef06dbfb9fd01f3809f884bf.

struct Params {
    width: u32,
    height: u32,
    size_x: u32,
    size_y: u32,
    size_z: u32,
    zero_width: u32,
    zero_height: u32,
    precedence: u32,
    sigma_s: f32,
    sigma_r: f32,
    threshold: f32,
    hue: f32,
    hue_denominator: f32,
    roi_x: i32,
    roi_y: i32,
    grid_x: i32,
    grid_y: i32,
    rescale: f32,
    pixel_count: u32,
    grid_cells: u32,
}

@group(0) @binding(0) var<storage, read> input_pixels: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> grid_a: array<atomic<u32>>;
@group(0) @binding(2) var<storage, read_write> grid_b: array<atomic<u32>>;
@group(0) @binding(3) var<storage, read_write> output_pixels: array<vec4<f32>>;
@group(0) @binding(4) var<uniform> params: Params;

var<workgroup> local_grid_index: array<i32, 256>;
var<workgroup> local_accumulator: array<vec4<f32>, 256>;

fn atomic_add_grid_a(index: u32, value: f32) {
    var old_bits = atomicLoad(&grid_a[index]);
    loop {
        let new_bits = bitcast<u32>(bitcast<f32>(old_bits) + value);
        let exchange = atomicCompareExchangeWeak(&grid_a[index], old_bits, new_bits);
        if exchange.exchanged {
            break;
        }
        old_bits = exchange.old_value;
    }
}

fn load_grid_a(cell: u32) -> vec4<f32> {
    let base = cell * 4u;
    return vec4<f32>(
        bitcast<f32>(atomicLoad(&grid_a[base])),
        bitcast<f32>(atomicLoad(&grid_a[base + 1u])),
        bitcast<f32>(atomicLoad(&grid_a[base + 2u])),
        bitcast<f32>(atomicLoad(&grid_a[base + 3u])),
    );
}

fn load_grid_b(cell: u32) -> vec4<f32> {
    let base = cell * 4u;
    return vec4<f32>(
        bitcast<f32>(atomicLoad(&grid_b[base])),
        bitcast<f32>(atomicLoad(&grid_b[base + 1u])),
        bitcast<f32>(atomicLoad(&grid_b[base + 2u])),
        bitcast<f32>(atomicLoad(&grid_b[base + 3u])),
    );
}

fn store_grid_a(cell: u32, value: vec4<f32>) {
    let base = cell * 4u;
    atomicStore(&grid_a[base], bitcast<u32>(value.x));
    atomicStore(&grid_a[base + 1u], bitcast<u32>(value.y));
    atomicStore(&grid_a[base + 2u], bitcast<u32>(value.z));
    atomicStore(&grid_a[base + 3u], bitcast<u32>(value.w));
}

fn store_grid_b(cell: u32, value: vec4<f32>) {
    let base = cell * 4u;
    atomicStore(&grid_b[base], bitcast<u32>(value.x));
    atomicStore(&grid_b[base + 1u], bitcast<u32>(value.y));
    atomicStore(&grid_b[base + 2u], bitcast<u32>(value.z));
    atomicStore(&grid_b[base + 3u], bitcast<u32>(value.w));
}

fn image_to_grid(point: vec3<f32>) -> vec3<f32> {
    return clamp(
        point / vec3<f32>(params.sigma_s, params.sigma_s, params.sigma_r),
        vec3<f32>(0.0),
        vec3<f32>(
            f32(params.size_x) - 1.0,
            f32(params.size_y) - 1.0,
            f32(params.size_z) - 1.0,
        ),
    );
}

fn add_accumulator(cell: i32, value: vec4<f32>) {
    let base = u32(cell) * 4u;
    atomic_add_grid_a(base, value.x);
    atomic_add_grid_a(base + 1u, value.y);
    atomic_add_grid_a(base + 2u, value.z);
    atomic_add_grid_a(base + 3u, value.w);
}

fn blur_b_to_a(base: u32, offset: u32, size: u32) {
    let weight_0 = 6.0 / 16.0;
    let weight_1 = 4.0 / 16.0;
    let weight_2 = 1.0 / 16.0;
    var index = base;
    var previous_2 = load_grid_b(index);
    var value = load_grid_b(index) * weight_0
        + load_grid_b(index + offset) * weight_1
        + load_grid_b(index + 2u * offset) * weight_2;
    store_grid_a(index, value);
    index += offset;
    var previous_1 = load_grid_b(index);
    value = load_grid_b(index) * weight_0
        + (load_grid_b(index + offset) + previous_2) * weight_1
        + load_grid_b(index + 2u * offset) * weight_2;
    store_grid_a(index, value);
    index += offset;
    var line = 2u;
    loop {
        if line >= size - 2u {
            break;
        }
        let current = load_grid_b(index);
        value = load_grid_b(index) * weight_0
            + (load_grid_b(index + offset) + previous_1) * weight_1
            + (load_grid_b(index + 2u * offset) + previous_2) * weight_2;
        store_grid_a(index, value);
        index += offset;
        previous_2 = previous_1;
        previous_1 = current;
        line += 1u;
    }
    let current = load_grid_b(index);
    value = load_grid_b(index) * weight_0
        + (load_grid_b(index + offset) + previous_1) * weight_1
        + previous_2 * weight_2;
    store_grid_a(index, value);
    index += offset;
    value = load_grid_b(index) * weight_0 + current * weight_1 + previous_1 * weight_2;
    store_grid_a(index, value);
}

fn blur_a_to_b(base: u32, offset: u32, size: u32) {
    let weight_0 = 6.0 / 16.0;
    let weight_1 = 4.0 / 16.0;
    let weight_2 = 1.0 / 16.0;
    var index = base;
    var previous_2 = load_grid_a(index);
    var value = load_grid_a(index) * weight_0
        + load_grid_a(index + offset) * weight_1
        + load_grid_a(index + 2u * offset) * weight_2;
    store_grid_b(index, value);
    index += offset;
    var previous_1 = load_grid_a(index);
    value = load_grid_a(index) * weight_0
        + (load_grid_a(index + offset) + previous_2) * weight_1
        + load_grid_a(index + 2u * offset) * weight_2;
    store_grid_b(index, value);
    index += offset;
    var line = 2u;
    loop {
        if line >= size - 2u {
            break;
        }
        let current = load_grid_a(index);
        value = load_grid_a(index) * weight_0
            + (load_grid_a(index + offset) + previous_1) * weight_1
            + (load_grid_a(index + 2u * offset) + previous_2) * weight_2;
        store_grid_b(index, value);
        index += offset;
        previous_2 = previous_1;
        previous_1 = current;
        line += 1u;
    }
    let current = load_grid_a(index);
    value = load_grid_a(index) * weight_0
        + (load_grid_a(index + offset) + previous_1) * weight_1
        + previous_2 * weight_2;
    store_grid_b(index, value);
    index += offset;
    value = load_grid_a(index) * weight_0 + current * weight_1 + previous_1 * weight_2;
    store_grid_b(index, value);
}

@compute @workgroup_size(16, 16, 1)
fn colorreconstruction_zero(@builtin(global_invocation_id) global_id: vec3<u32>) {
    if global_id.x >= params.zero_width || global_id.y >= params.zero_height {
        return;
    }
    atomicStore(&grid_a[global_id.y * params.zero_width + global_id.x], 0u);
}

@compute @workgroup_size(16, 16, 1)
fn colorreconstruction_splat(
    @builtin(global_invocation_id) global_id: vec3<u32>,
    @builtin(local_invocation_id) local_id: vec3<u32>,
) {
    let local_index = local_id.y * 16u + local_id.x;
    if global_id.x < params.width && global_id.y < params.height {
        let pixel = input_pixels[global_id.y * params.width + global_id.x];
        var weight = 1.0;
        if params.precedence == 1u {
            weight = sqrt(pixel.y * pixel.y + pixel.z * pixel.z);
        } else if params.precedence == 2u {
            var difference = atan2(pixel.z, pixel.y) - params.hue;
            if difference > 3.14159265358979323846 {
                difference -= 6.28318530717958647693;
            } else if difference < -3.14159265358979323846 {
                difference += 6.28318530717958647693;
            }
            weight = exp(-difference * difference / params.hue_denominator);
        }
        let grid_point = image_to_grid(vec3<f32>(
            f32(global_id.x),
            f32(global_id.y),
            pixel.x,
        ));
        let integer_point = vec3<u32>(round(grid_point));
        local_grid_index[local_index] = i32(
            integer_point.x
                + params.size_x * integer_point.y
                + params.size_x * params.size_y * integer_point.z,
        );
        local_accumulator[local_index] = select(
            vec4<f32>(0.0),
            weight * vec4<f32>(pixel.xyz, 1.0),
            pixel.x < params.threshold,
        );
    } else {
        local_grid_index[local_index] = -1;
        local_accumulator[local_index] = vec4<f32>(0.0);
    }

    workgroupBarrier();
    if local_id.x != 0u {
        return;
    }

    var line_index = local_id.y * 16u;
    var old_grid_index = local_grid_index[line_index];
    var accumulated = local_accumulator[line_index];
    var column = 1u;
    loop {
        if column >= 16u || old_grid_index == -1 {
            break;
        }
        line_index = local_id.y * 16u + column;
        if local_grid_index[line_index] != old_grid_index {
            add_accumulator(old_grid_index, accumulated);
            old_grid_index = local_grid_index[line_index];
            accumulated = local_accumulator[line_index];
        } else {
            accumulated = local_accumulator[line_index];
        }
        column += 1u;
    }
    if old_grid_index != -1 {
        add_accumulator(old_grid_index, accumulated);
    }
}

@compute @workgroup_size(16, 16, 1)
fn colorreconstruction_blur_x(@builtin(global_invocation_id) global_id: vec3<u32>) {
    if global_id.x >= params.size_z || global_id.y >= params.size_y {
        return;
    }
    let base = global_id.x * params.size_x * params.size_y + global_id.y * params.size_x;
    blur_b_to_a(base, 1u, params.size_x);
}

@compute @workgroup_size(16, 16, 1)
fn colorreconstruction_blur_y(@builtin(global_invocation_id) global_id: vec3<u32>) {
    if global_id.x >= params.size_z || global_id.y >= params.size_x {
        return;
    }
    let base = global_id.x * params.size_x * params.size_y + global_id.y;
    blur_a_to_b(base, params.size_x, params.size_y);
}

@compute @workgroup_size(16, 16, 1)
fn colorreconstruction_blur_z(@builtin(global_invocation_id) global_id: vec3<u32>) {
    if global_id.x >= params.size_x || global_id.y >= params.size_y {
        return;
    }
    let base = global_id.x + global_id.y * params.size_x;
    blur_b_to_a(base, params.size_x * params.size_y, params.size_z);
}

@compute @workgroup_size(16, 16, 1)
fn colorreconstruction_slice(@builtin(global_invocation_id) global_id: vec3<u32>) {
    if global_id.x >= params.width || global_id.y >= params.height {
        return;
    }
    let pixel_index = global_id.y * params.width + global_id.x;
    var pixel = input_pixels[pixel_index];
    let blend = clamp(20.0 / params.threshold * pixel.x - 19.0, 0.0, 1.0);
    let rescaled = vec2<f32>(
        f32(params.roi_x + i32(global_id.x)) * params.rescale - f32(params.grid_x),
        f32(params.roi_y + i32(global_id.y)) * params.rescale - f32(params.grid_y),
    );
    let grid_point = image_to_grid(vec3<f32>(rescaled, pixel.x));
    let integer_point = min(
        vec3<u32>(grid_point),
        vec3<u32>(params.size_x - 2u, params.size_y - 2u, params.size_z - 2u),
    );
    let fraction = grid_point - vec3<f32>(integer_point);
    let offset_x = 1u;
    let offset_y = params.size_x;
    let offset_z = params.size_y * params.size_x;
    let grid_index = integer_point.x
        + params.size_x * (integer_point.y + params.size_y * integer_point.z);
    let sliced =
        load_grid_a(grid_index) * (1.0 - fraction.x) * (1.0 - fraction.y) * (1.0 - fraction.z)
        + load_grid_a(grid_index + offset_x) * fraction.x * (1.0 - fraction.y) * (1.0 - fraction.z)
        + load_grid_a(grid_index + offset_y) * (1.0 - fraction.x) * fraction.y * (1.0 - fraction.z)
        + load_grid_a(grid_index + offset_x + offset_y) * fraction.x * fraction.y * (1.0 - fraction.z)
        + load_grid_a(grid_index + offset_z) * (1.0 - fraction.x) * (1.0 - fraction.y) * fraction.z
        + load_grid_a(grid_index + offset_x + offset_z) * fraction.x * (1.0 - fraction.y) * fraction.z
        + load_grid_a(grid_index + offset_y + offset_z) * (1.0 - fraction.x) * fraction.y * fraction.z
        + load_grid_a(grid_index + offset_x + offset_y + offset_z) * fraction.x * fraction.y * fraction.z;
    let sliced_lightness = max(sliced.x, 0.01);
    if sliced.w > 0.0 {
        pixel.y = pixel.y * (1.0 - blend) + sliced.y * pixel.x / sliced_lightness * blend;
        pixel.z = pixel.z * (1.0 - blend) + sliced.z * pixel.x / sliced_lightness * blend;
    }
    output_pixels[pixel_index] = pixel;
}
