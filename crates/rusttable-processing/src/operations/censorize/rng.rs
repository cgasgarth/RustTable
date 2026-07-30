//! The scalar splitmix32/xoshiro128+/Box-Muller sequence used by censorize.

#![allow(
    clippy::cast_precision_loss,
    clippy::many_single_char_names,
    clippy::missing_panics_doc,
    clippy::should_implement_trait
)]

pub const CENSORIZE_RNG_VERSION: &str = "splitmix32-xoshiro128plus-box-muller.v1";

#[must_use]
pub fn splitmix32(seed: u64) -> u32 {
    let mut result = (seed ^ (seed >> 33)).wrapping_mul(0x62a9_d9ed_7997_05f5);
    result = (result ^ (result >> 28)).wrapping_mul(0xcb24_d0a5_c88c_35b3);
    (result >> 32) as u32
}

#[must_use]
pub fn xoshiro128plus(state: &mut [u32; 4]) -> f32 {
    let result = state[0].wrapping_add(state[3]);
    let t = state[1] << 9;
    state[2] ^= state[0];
    state[3] ^= state[1];
    state[1] ^= state[2];
    state[0] ^= state[3];
    state[2] ^= t;
    state[3] = state[3].rotate_left(11);
    (result >> 8) as f32 * (1.0 / 16_777_216.0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CensorizeRng {
    state: [u32; 4],
}

impl CensorizeRng {
    #[must_use]
    pub fn for_pixel(x: usize, y: usize) -> Self {
        let x = u64::try_from(x).expect("pixel coordinate fits u64");
        let y = u64::try_from(y).expect("pixel coordinate fits u64");
        let mut rng = Self {
            state: [
                splitmix32(x + 1),
                splitmix32((x + 1).wrapping_mul(y + 3)),
                splitmix32(1337),
                splitmix32(666),
            ],
        };
        for _ in 0..4 {
            let _ = rng.next();
        }
        rng
    }
    #[must_use]
    pub fn next(&mut self) -> f32 {
        xoshiro128plus(&mut self.state)
    }
    #[must_use]
    pub fn gaussian(&mut self, mu: f32, sigma: f32, flip: bool) -> f32 {
        let u1 = self.next().max(f32::MIN_POSITIVE);
        let u2 = self.next();
        let angle = 2.0 * std::f32::consts::PI * u2;
        let unit = (-2.0 * u1.ln()).sqrt() * if flip { angle.cos() } else { angle.sin() };
        unit * sigma + mu
    }
}

#[must_use]
pub fn gaussian_noise(mu: f32, sigma: f32, flip: bool, state: &mut [u32; 4]) -> f32 {
    let mut rng = CensorizeRng { state: *state };
    let value = rng.gaussian(mu, sigma, flip);
    *state = rng.state;
    value
}
