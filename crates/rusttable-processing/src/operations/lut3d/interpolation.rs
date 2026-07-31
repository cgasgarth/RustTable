//! Native LUT3D interpolation equations ported from `src/iop/lut3d.c` and
//! `data/kernels/lut3d.cl`.

#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]

use super::codec::Lut3dInterpolation;
use super::parser::Lut3d;

impl Lut3d {
    /// Samples RGB through the native R-fastest CLUT and preserves alpha.
    #[must_use]
    pub fn sample(&self, input: [f32; 4], interpolation: Lut3dInterpolation) -> [f32; 4] {
        let coordinate = [
            clip(input[0]) * (self.level - 1) as f32,
            clip(input[1]) * (self.level - 1) as f32,
            clip(input[2]) * (self.level - 1) as f32,
        ];
        let level_minus_two = self.level - 2;
        let mut base = [
            coordinate[0] as usize,
            coordinate[1] as usize,
            coordinate[2] as usize,
        ];
        for axis in &mut base {
            *axis = (*axis).min(level_minus_two);
        }
        let delta = [
            coordinate[0] - base[0] as f32,
            coordinate[1] - base[1] as f32,
            coordinate[2] - base[2] as f32,
        ];

        let p000 = self.corner(base, 0, 0, 0);
        let p100 = self.corner(base, 1, 0, 0);
        let p010 = self.corner(base, 0, 1, 0);
        let p110 = self.corner(base, 1, 1, 0);
        let p001 = self.corner(base, 0, 0, 1);
        let p101 = self.corner(base, 1, 0, 1);
        let p011 = self.corner(base, 0, 1, 1);
        let p111 = self.corner(base, 1, 1, 1);

        let rgb = match interpolation {
            Lut3dInterpolation::Trilinear => {
                trilinear(p000, p100, p010, p110, p001, p101, p011, p111, delta)
            }
            Lut3dInterpolation::Tetrahedral => {
                tetrahedral(p000, p100, p010, p110, p001, p101, p011, p111, delta)
            }
            Lut3dInterpolation::Pyramid => {
                pyramid(p000, p100, p010, p110, p001, p101, p011, p111, delta)
            }
        };
        [rgb[0], rgb[1], rgb[2], input[3]]
    }

    fn corner(&self, base: [usize; 3], red: usize, green: usize, blue: usize) -> [f32; 3] {
        let index = (base[0] + red)
            + self.level * (base[1] + green)
            + self.level * self.level * (base[2] + blue);
        self.value(index)
    }
}

fn clip(value: f32) -> f32 {
    // Native CLIP is NaN-safe and maps NaN to zero.
    if value.is_nan() || value < 0.0 {
        0.0
    } else if value > 1.0 {
        1.0
    } else {
        value
    }
}

fn blend(a: [f32; 3], b: [f32; 3], amount: f32) -> [f32; 3] {
    [
        a[0] * (1.0 - amount) + b[0] * amount,
        a[1] * (1.0 - amount) + b[1] * amount,
        a[2] * (1.0 - amount) + b[2] * amount,
    ]
}

#[allow(clippy::too_many_arguments)]
fn trilinear(
    p000: [f32; 3],
    p100: [f32; 3],
    p010: [f32; 3],
    p110: [f32; 3],
    p001: [f32; 3],
    p101: [f32; 3],
    p011: [f32; 3],
    p111: [f32; 3],
    delta: [f32; 3],
) -> [f32; 3] {
    // Keep the native plane/axis nesting and operation order.
    let tmp1 = blend(p000, p100, delta[0]);
    let tmp2 = blend(p010, p110, delta[0]);
    let output = blend(tmp1, tmp2, delta[1]);
    let tmp1 = blend(p001, p101, delta[0]);
    let tmp2 = blend(p011, p111, delta[0]);
    let tmp1 = blend(tmp1, tmp2, delta[1]);
    blend(output, tmp1, delta[2])
}

#[allow(clippy::too_many_arguments)]
fn tetrahedral(
    p000: [f32; 3],
    p100: [f32; 3],
    p010: [f32; 3],
    p110: [f32; 3],
    p001: [f32; 3],
    p101: [f32; 3],
    p011: [f32; 3],
    p111: [f32; 3],
    delta: [f32; 3],
) -> [f32; 3] {
    let [dr, dg, db] = delta;
    if dr > dg {
        if dg > db {
            weighted4(p000, 1.0 - dr, p100, dr - dg, p110, dg - db, p111, db)
        } else if dr > db {
            weighted4(p000, 1.0 - dr, p100, dr - db, p101, db - dg, p111, dg)
        } else {
            weighted4(p000, 1.0 - db, p001, db - dr, p101, dr - dg, p111, dg)
        }
    } else if db > dg {
        weighted4(p000, 1.0 - db, p001, db - dg, p011, dg - dr, p111, dr)
    } else if db > dr {
        weighted4(p000, 1.0 - dg, p010, dg - db, p011, db - dr, p111, dr)
    } else {
        weighted4(p000, 1.0 - dg, p010, dg - dr, p110, dr - db, p111, db)
    }
}

fn weighted4(
    a: [f32; 3],
    wa: f32,
    b: [f32; 3],
    wb: f32,
    c: [f32; 3],
    wc: f32,
    d: [f32; 3],
    wd: f32,
) -> [f32; 3] {
    [
        wa * a[0] + wb * b[0] + wc * c[0] + wd * d[0],
        wa * a[1] + wb * b[1] + wc * c[1] + wd * d[1],
        wa * a[2] + wb * b[2] + wc * c[2] + wd * d[2],
    ]
}

#[allow(clippy::too_many_arguments)]
fn pyramid(
    p000: [f32; 3],
    p100: [f32; 3],
    p010: [f32; 3],
    p110: [f32; 3],
    p001: [f32; 3],
    p101: [f32; 3],
    p011: [f32; 3],
    p111: [f32; 3],
    delta: [f32; 3],
) -> [f32; 3] {
    let [dr, dg, db] = delta;
    if dg > dr && db > dr {
        pyramid_first(p000, p001, p010, p011, p111, dr, dg, db)
    } else if dr > dg && db > dg {
        pyramid_second(p000, p001, p100, p101, p111, dr, dg, db)
    } else {
        pyramid_third(p000, p010, p100, p110, p111, dr, dg, db)
    }
}

fn pyramid_first(
    p000: [f32; 3],
    p001: [f32; 3],
    p010: [f32; 3],
    p011: [f32; 3],
    p111: [f32; 3],
    dr: f32,
    dg: f32,
    db: f32,
) -> [f32; 3] {
    [
        p000[0]
            + (p111[0] - p011[0]) * dr
            + (p010[0] - p000[0]) * dg
            + (p001[0] - p000[0]) * db
            + (p011[0] - p001[0] - p010[0] + p000[0]) * dg * db,
        p000[1]
            + (p111[1] - p011[1]) * dr
            + (p010[1] - p000[1]) * dg
            + (p001[1] - p000[1]) * db
            + (p011[1] - p001[1] - p010[1] + p000[1]) * dg * db,
        p000[2]
            + (p111[2] - p011[2]) * dr
            + (p010[2] - p000[2]) * dg
            + (p001[2] - p000[2]) * db
            + (p011[2] - p001[2] - p010[2] + p000[2]) * dg * db,
    ]
}

fn pyramid_second(
    p000: [f32; 3],
    p001: [f32; 3],
    p100: [f32; 3],
    p101: [f32; 3],
    p111: [f32; 3],
    dr: f32,
    dg: f32,
    db: f32,
) -> [f32; 3] {
    [
        p000[0]
            + (p100[0] - p000[0]) * dr
            + (p111[0] - p101[0]) * dg
            + (p001[0] - p000[0]) * db
            + (p101[0] - p001[0] - p100[0] + p000[0]) * dr * db,
        p000[1]
            + (p100[1] - p000[1]) * dr
            + (p111[1] - p101[1]) * dg
            + (p001[1] - p000[1]) * db
            + (p101[1] - p001[1] - p100[1] + p000[1]) * dr * db,
        p000[2]
            + (p100[2] - p000[2]) * dr
            + (p111[2] - p101[2]) * dg
            + (p001[2] - p000[2]) * db
            + (p101[2] - p001[2] - p100[2] + p000[2]) * dr * db,
    ]
}

fn pyramid_third(
    p000: [f32; 3],
    p010: [f32; 3],
    p100: [f32; 3],
    p110: [f32; 3],
    p111: [f32; 3],
    dr: f32,
    dg: f32,
    db: f32,
) -> [f32; 3] {
    [
        p000[0]
            + (p100[0] - p000[0]) * dr
            + (p010[0] - p000[0]) * dg
            + (p111[0] - p110[0]) * db
            + (p110[0] - p100[0] - p010[0] + p000[0]) * dr * dg,
        p000[1]
            + (p100[1] - p000[1]) * dr
            + (p010[1] - p000[1]) * dg
            + (p111[1] - p110[1]) * db
            + (p110[1] - p100[1] - p010[1] + p000[1]) * dr * dg,
        p000[2]
            + (p100[2] - p000[2]) * dr
            + (p010[2] - p000[2]) * dg
            + (p111[2] - p110[2]) * db
            + (p110[2] - p100[2] - p010[2] + p000[2]) * dr * dg,
    ]
}
