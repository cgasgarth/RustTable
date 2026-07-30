struct GrainParams {
  pixel_count: u32,
  width: u32,
  height: u32,
  channel: u32,
  full_width: u32,
  full_height: u32,
  origin_x: u32,
  origin_y: u32,
  seed_low: u32,
  seed_high: u32,
  zoom: f32,
  strength: f32,
  midtones_bias: f32,
  _reserved: vec3<f32>,
}

@group(0) @binding(0) var<storage, read> input_pixels: array<f32>;
@group(0) @binding(1) var<storage, read_write> output_pixels: array<f32>;
@group(0) @binding(2) var<uniform> params: GrainParams;
@group(0) @binding(3) var<storage, read> grain_lut: array<f32>;

fn grain_hash(seed_low: u32, seed_high: u32, x: i32, y: i32, channel: u32, octave: u32) -> u32 {
  var value = seed_low ^ seed_high ^ (u32(x) * 0x9e3779b9u) ^ (u32(y) * 0x85ebca6bu);
  value = value ^ (channel * 0xc2b2ae35u) ^ (octave * 0x27d4eb2du);
  value = value ^ (value >> 16u);
  value = value * 0x7feb352du;
  value = value ^ (value >> 15u);
  value = value * 0x846ca68bu;
  return value ^ (value >> 16u);
}

fn hash_to_unit(value: u32) -> f32 {
  return f32(value >> 8u) / 16777216.0;
}

fn signed_hash(x: i32, y: i32, channel: u32, octave: u32) -> f32 {
  return hash_to_unit(grain_hash(params.seed_low, params.seed_high, x, y, channel, octave)) * 2.0 - 1.0;
}

fn value_noise(x: f32, y: f32, channel: u32, octave: u32) -> f32 {
  let x_floor = floor(x);
  let y_floor = floor(y);
  let x0 = i32(x_floor);
  let y0 = i32(y_floor);
  let fx = x - x_floor;
  let fy = y - y_floor;
  let sx = fx * fx * (3.0 - 2.0 * fx);
  let sy = fy * fy * (3.0 - 2.0 * fy);
  let a = signed_hash(x0, y0, channel, octave);
  let b = signed_hash(x0 + 1, y0, channel, octave);
  let c = signed_hash(x0, y0 + 1, channel, octave);
  let d = signed_hash(x0 + 1, y0 + 1, channel, octave);
  let low = a + (b - a) * sx;
  let high = c + (d - c) * sx;
  return low + (high - low) * sy;
}

fn grain_noise(x: f32, y: f32, channel: u32) -> f32 {
  let first = 0.2340 * value_noise(x / params.zoom * 0.4910, y / params.zoom * 0.4910, channel, 0u);
  let second = 0.7850 * value_noise(x / params.zoom * 0.9441, y / params.zoom * 0.9441, channel, 1u);
  let third = 1.2150 * value_noise(x / params.zoom * 1.7280, y / params.zoom * 1.7280, channel, 2u);
  return (first + second + third) / 2.234;
}

fn response(noise: f32, luminance: f32, scale: f32) -> f32 {
  let sample = clamp((noise * params.strength * scale + 0.5) * 127.0, 0.0, 127.0);
  let y = clamp(luminance * 127.0, 0.0, 127.0);
  let x0 = min(u32(sample), 126u);
  let y0 = min(u32(y), 126u);
  let xd = sample - f32(x0);
  let yd = y - f32(y0);
  let low = grain_lut[y0 * 128u + x0] * (1.0 - xd) + grain_lut[y0 * 128u + x0 + 1u] * xd;
  let high = grain_lut[(y0 + 1u) * 128u + x0] * (1.0 - xd) + grain_lut[(y0 + 1u) * 128u + x0 + 1u] * xd;
  return (low * (1.0 - yd) + high * yd) / 100.0;
}

fn rgb_to_hsl(rgb: vec3<f32>) -> vec3<f32> {
  let maximum = max(max(rgb.r, rgb.g), rgb.b);
  let minimum = min(min(rgb.r, rgb.g), rgb.b);
  let lightness = (maximum + minimum) * 0.5;
  let delta = maximum - minimum;
  if (delta == 0.0) { return vec3<f32>(0.0, 0.0, lightness); }
  let saturation = delta / max(1.0 - abs(2.0 * lightness - 1.0), 0.000001);
  var hue = 0.0;
  if (maximum == rgb.r) {
    hue = ((rgb.g - rgb.b) / delta) % 6.0;
    if (hue < 0.0) { hue = hue + 6.0; }
  } else if (maximum == rgb.g) {
    hue = (rgb.b - rgb.r) / delta + 2.0;
  } else {
    hue = (rgb.r - rgb.g) / delta + 4.0;
  }
  return vec3<f32>(hue / 6.0, saturation, lightness);
}

fn hsl_to_rgb(hsl: vec3<f32>) -> vec3<f32> {
  let chroma = (1.0 - abs(2.0 * hsl.b - 1.0)) * hsl.g;
  var hue = hsl.r * 6.0;
  hue = hue - floor(hue / 2.0) * 2.0;
  let x = chroma * (1.0 - abs(hue - 1.0));
  let m = hsl.b - chroma * 0.5;
  let section = floor(hsl.r * 6.0);
  var rgb = vec3<f32>(chroma, x, 0.0);
  if (section == 1.0) { rgb = vec3<f32>(x, chroma, 0.0); }
  else if (section == 2.0) { rgb = vec3<f32>(0.0, chroma, x); }
  else if (section == 3.0) { rgb = vec3<f32>(0.0, x, chroma); }
  else if (section == 4.0) { rgb = vec3<f32>(x, 0.0, chroma); }
  else if (section >= 5.0) { rgb = vec3<f32>(chroma, 0.0, x); }
  return rgb + vec3<f32>(m);
}

@compute @workgroup_size(256, 1, 1)
fn grain_point(@builtin(global_invocation_id) id: vec3<u32>) {
  if (id.x >= params.pixel_count) { return; }
  let local_x = id.x % params.width;
  let local_y = id.x / params.width;
  let x = f32(params.origin_x + local_x) + 0.5;
  let y = f32(params.origin_y + local_y) + 0.5;
  let base = id.x * 4u;
  let pixel = vec4<f32>(input_pixels[base], input_pixels[base + 1u], input_pixels[base + 2u], input_pixels[base + 3u]);
  let luminance = clamp(0.2126 * pixel.r + 0.7152 * pixel.g + 0.0722 * pixel.b, 0.0, 1.0);
  var rgb = pixel.rgb;
  if (params.channel == 2u) {
    rgb = rgb + vec3<f32>(response(grain_noise(x, y, 2u), luminance, 0.15));
  } else if (params.channel == 3u) {
    rgb = rgb + vec3<f32>(
      response(grain_noise(x, y, 0u), luminance, 0.25),
      response(grain_noise(x, y, 1u), luminance, 0.25),
      response(grain_noise(x, y, 2u), luminance, 0.25));
  } else {
    var hsl = rgb_to_hsl(rgb);
    let delta = response(grain_noise(x, y, params.channel), luminance, 0.25) * 0.01;
    if (params.channel == 0u) {
      hsl.x = hsl.x + delta;
      hsl.x = hsl.x - floor(hsl.x);
    } else {
      hsl.y = hsl.y + delta;
    }
    rgb = hsl_to_rgb(hsl);
  }
  output_pixels[base] = rgb.r;
  output_pixels[base + 1u] = rgb.g;
  output_pixels[base + 2u] = rgb.b;
  output_pixels[base + 3u] = pixel.a;
}
