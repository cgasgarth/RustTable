// Safe WGPU port of Darktable's data/kernels/bilateral.cl.
//
// The grid is x-fastest, matching the OpenCL path:
// x + size_x * (y + size_y * z).

struct BilateralParams {
    width: u32,
    height: u32,
    size_x: u32,
    size_y: u32,
    size_z: u32,
    sigma_s: f32,
    sigma_r: f32,
    detail: f32,
    offset1: u32,
    offset2: u32,
    offset3: u32,
    size1: u32,
    size2: u32,
    size3: u32,
    pixel_count: u32,
    grid_values: u32,
}

@group(0) @binding(0) var<storage, read_write> zero_grid: array<atomic<u32>>;
@group(0) @binding(1) var<uniform> params: BilateralParams;
@group(0) @binding(2) var<storage, read> splat_input: array<f32>;
@group(0) @binding(3) var<storage, read_write> splat_grid: array<atomic<u32>>;
@group(0) @binding(4) var<storage, read> blur_input: array<f32>;
@group(0) @binding(5) var<storage, read_write> blur_output: array<f32>;
@group(0) @binding(6) var<storage, read> slice_input: array<f32>;
@group(0) @binding(7) var<storage, read> slice_target: array<f32>;
@group(0) @binding(8) var<storage, read_write> slice_output: array<f32>;
@group(0) @binding(9) var<storage, read> slice_grid: array<f32>;

var<workgroup> splat_indices: array<u32, 256>;
var<workgroup> splat_accum: array<f32, 2048>;

const INVALID_GRID_INDEX: u32 = 0xffffffffu;

fn atomic_add_f32(index: u32, value: f32) {
    var old_bits = atomicLoad(&splat_grid[index]);
    loop {
        let new_bits = bitcast<u32>(bitcast<f32>(old_bits) + value);
        let result = atomicCompareExchangeWeak(&splat_grid[index], old_bits, new_bits);
        if result.exchanged {
            break;
        }
        old_bits = result.old_value;
    }
}

fn flush_splat(base: u32, values: array<f32, 8>) {
    let offset_x = 1u;
    let offset_y = params.size_x;
    let offset_z = params.size_x * params.size_y;
    atomic_add_f32(base, values[0]);
    atomic_add_f32(base + offset_x, values[1]);
    atomic_add_f32(base + offset_y, values[2]);
    atomic_add_f32(base + offset_y + offset_x, values[3]);
    atomic_add_f32(base + offset_z, values[4]);
    atomic_add_f32(base + offset_z + offset_x, values[5]);
    atomic_add_f32(base + offset_z + offset_y, values[6]);
    atomic_add_f32(base + offset_z + offset_y + offset_x, values[7]);
}

@compute @workgroup_size(16, 16, 1)
fn zero(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let x = global_id.x;
    let y = global_id.y;
    let zero_height = params.size_y * params.size_z;
    if x < params.size_x && y < zero_height {
        let index = x + params.size_x * y;
        atomicStore(&zero_grid[index], 0u);
    }
}

@compute @workgroup_size(16, 16, 1)
fn splat(
    @builtin(global_invocation_id) global_id: vec3<u32>,
    @builtin(local_invocation_id) local_id: vec3<u32>,
) {
    let local_index = local_id.y * 16u + local_id.x;
    let accum_index = local_index * 8u;
    for (var contribution = 0u; contribution < 8u; contribution++) {
        splat_accum[accum_index + contribution] = 0.0;
    }

    let x = global_id.x;
    let y = global_id.y;
    if x < params.width && y < params.height {
        let pixel_index = y * params.width + x;
        let lightness = splat_input[pixel_index * 4u];
        let grid_position = clamp(
            vec3<f32>(
                f32(x) / params.sigma_s,
                f32(y) / params.sigma_s,
                lightness / params.sigma_r,
            ),
            vec3<f32>(0.0),
            vec3<f32>(
                f32(params.size_x - 1u),
                f32(params.size_y - 1u),
                f32(params.size_z - 1u),
            ),
        );
        let base_coordinates = min(
            vec3<u32>(grid_position),
            vec3<u32>(params.size_x - 2u, params.size_y - 2u, params.size_z - 2u),
        );
        let fraction = grid_position - vec3<f32>(base_coordinates);
        let base = base_coordinates.x
            + params.size_x * (base_coordinates.y + params.size_y * base_coordinates.z);
        splat_indices[local_index] = base;

        let scale = 100.0 / (params.sigma_s * params.sigma_s);
        let near_x = 1.0 - fraction.x;
        let near_y = 1.0 - fraction.y;
        let near_z = 1.0 - fraction.z;
        splat_accum[accum_index] = scale * near_x * near_y * near_z;
        splat_accum[accum_index + 1u] = scale * fraction.x * near_y * near_z;
        splat_accum[accum_index + 2u] = scale * near_x * fraction.y * near_z;
        splat_accum[accum_index + 3u] = scale * fraction.x * fraction.y * near_z;
        splat_accum[accum_index + 4u] = scale * near_x * near_y * fraction.z;
        splat_accum[accum_index + 5u] = scale * fraction.x * near_y * fraction.z;
        splat_accum[accum_index + 6u] = scale * near_x * fraction.y * fraction.z;
        splat_accum[accum_index + 7u] =
            scale * fraction.x * fraction.y * fraction.z;
    } else {
        splat_indices[local_index] = INVALID_GRID_INDEX;
    }

    // Every lane, including padded lanes, must reach this barrier.
    workgroupBarrier();

    // As in the retained kernel, the first lane in each row merges only
    // consecutive pixels that splat to the same base cell.
    if local_id.x != 0u {
        return;
    }
    let row_start = local_id.y * 16u;
    var old_base = splat_indices[row_start];
    if old_base == INVALID_GRID_INDEX {
        return;
    }
    var values: array<f32, 8>;
    for (var contribution = 0u; contribution < 8u; contribution++) {
        values[contribution] = splat_accum[row_start * 8u + contribution];
    }
    for (var lane = 1u; lane < 16u; lane++) {
        let lane_index = row_start + lane;
        let base = splat_indices[lane_index];
        if base != old_base {
            flush_splat(old_base, values);
            old_base = base;
            if old_base == INVALID_GRID_INDEX {
                return;
            }
            for (var contribution = 0u; contribution < 8u; contribution++) {
                values[contribution] = splat_accum[lane_index * 8u + contribution];
            }
        } else {
            for (var contribution = 0u; contribution < 8u; contribution++) {
                values[contribution] += splat_accum[lane_index * 8u + contribution];
            }
        }
    }
    flush_splat(old_base, values);
}

fn blur_sample(line_base: u32, position: u32, relative: i32) -> f32 {
    var sample_position = position;
    if relative < 0 {
        let distance = u32(-relative);
        if position < distance {
            return 0.0;
        }
        sample_position = position - distance;
    } else {
        let distance = u32(relative);
        if distance >= params.size3 - position {
            return 0.0;
        }
        sample_position = position + distance;
    }
    return blur_input[line_base + sample_position * params.offset3];
}

@compute @workgroup_size(8, 8, 1)
fn blur_line(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let position = global_id.x;
    let line = global_id.y;
    let outer = global_id.z;
    if position >= params.size3 || line >= params.size2 || outer >= params.size1 {
        return;
    }
    let line_base = outer * params.offset1 + line * params.offset2;
    let output_index = line_base + position * params.offset3;
    let center = blur_sample(line_base, position, 0);
    let left1 = blur_sample(line_base, position, -1);
    let right1 = blur_sample(line_base, position, 1);
    let left2 = blur_sample(line_base, position, -2);
    let right2 = blur_sample(line_base, position, 2);
    blur_output[output_index] =
        (6.0 * center + 4.0 * (left1 + right1) + left2 + right2) / 16.0;
}

@compute @workgroup_size(8, 8, 1)
fn blur_line_z(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let position = global_id.x;
    let line = global_id.y;
    let outer = global_id.z;
    if position >= params.size3 || line >= params.size2 || outer >= params.size1 {
        return;
    }
    let line_base = outer * params.offset1 + line * params.offset2;
    let output_index = line_base + position * params.offset3;
    let left1 = blur_sample(line_base, position, -1);
    let right1 = blur_sample(line_base, position, 1);
    let left2 = blur_sample(line_base, position, -2);
    let right2 = blur_sample(line_base, position, 2);
    blur_output[output_index] =
        (4.0 * (right1 - left1) + 2.0 * (right2 - left2)) / 16.0;
}

fn interpolated_grid(x: u32, y: u32, lightness: f32) -> f32 {
    let grid_position = clamp(
        vec3<f32>(
            f32(x) / params.sigma_s,
            f32(y) / params.sigma_s,
            lightness / params.sigma_r,
        ),
        vec3<f32>(0.0),
        vec3<f32>(
            f32(params.size_x - 1u),
            f32(params.size_y - 1u),
            f32(params.size_z - 1u),
        ),
    );
    let base_coordinates = min(
        vec3<u32>(grid_position),
        vec3<u32>(params.size_x - 2u, params.size_y - 2u, params.size_z - 2u),
    );
    let fraction = grid_position - vec3<f32>(base_coordinates);
    let near_x = 1.0 - fraction.x;
    let near_y = 1.0 - fraction.y;
    let near_z = 1.0 - fraction.z;
    let offset_x = 1u;
    let offset_y = params.size_x;
    let offset_z = params.size_x * params.size_y;
    let base = base_coordinates.x
        + params.size_x * (base_coordinates.y + params.size_y * base_coordinates.z);
    return slice_grid[base] * near_x * near_y * near_z
        + slice_grid[base + offset_x] * fraction.x * near_y * near_z
        + slice_grid[base + offset_y] * near_x * fraction.y * near_z
        + slice_grid[base + offset_y + offset_x] * fraction.x * fraction.y * near_z
        + slice_grid[base + offset_z] * near_x * near_y * fraction.z
        + slice_grid[base + offset_z + offset_x] * fraction.x * near_y * fraction.z
        + slice_grid[base + offset_z + offset_y] * near_x * fraction.y * fraction.z
        + slice_grid[base + offset_z + offset_y + offset_x]
            * fraction.x * fraction.y * fraction.z;
}

fn write_slice(global_id: vec3<u32>, additive: bool) {
    let x = global_id.x;
    let y = global_id.y;
    if x >= params.width || y >= params.height {
        return;
    }
    let pixel = y * params.width + x;
    let component = pixel * 4u;
    let guide_lightness = slice_input[component];
    let correction =
        -params.detail * params.sigma_r * 0.04 * interpolated_grid(x, y, guide_lightness);

    var output_lightness = guide_lightness;
    var output_a = slice_input[component + 1u];
    var output_b = slice_input[component + 2u];
    var output_alpha = slice_input[component + 3u];
    if additive {
        output_lightness = slice_target[component];
        output_a = slice_target[component + 1u];
        output_b = slice_target[component + 2u];
        output_alpha = slice_target[component + 3u];
    }
    slice_output[component] = max(0.0, output_lightness + correction);
    slice_output[component + 1u] = output_a;
    slice_output[component + 2u] = output_b;
    slice_output[component + 3u] = output_alpha;
}

@compute @workgroup_size(16, 16, 1)
fn slice(@builtin(global_invocation_id) global_id: vec3<u32>) {
    write_slice(global_id, false);
}

@compute @workgroup_size(16, 16, 1)
fn slice_to_output(@builtin(global_invocation_id) global_id: vec3<u32>) {
    write_slice(global_id, true);
}
