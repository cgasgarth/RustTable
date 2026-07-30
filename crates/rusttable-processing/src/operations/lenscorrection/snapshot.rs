//! Compact generated-compatible records selected from Lensfun's database.
//!
//! Source: Lensfun `data/db` at [`LENSFUN_DATABASE_COMMIT`]. The upstream
//! database records are CC BY-SA 3.0; these values retain that provenance and
//! are kept in `RustTable` so runtime behavior does not depend on host files.

use super::parameters::LensGeometry;

/// The Lensfun data commit used to generate this compact Rust snapshot.
pub const LENSFUN_DATABASE_COMMIT: &str = "698a39eea69be00f4f25b6da6c1ad34b1f162b50";
pub const LENSFUN_DATABASE_TIMESTAMP: i64 = 1_577_948_414;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraProfile {
    pub maker: &'static str,
    pub model: &'static str,
    pub aliases: &'static [&'static str],
    pub mount: &'static str,
    pub crop_factor: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DistortionCalibration {
    Poly3 { focal: f32, k1: f32 },
    PtLens { focal: f32, a: f32, b: f32, c: f32 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TcaCalibration {
    pub focal: f32,
    pub red_linear: f32,
    pub red_cubic: f32,
    pub blue_linear: f32,
    pub blue_cubic: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VignettingCalibration {
    pub focal: f32,
    pub aperture: f32,
    pub distance: f32,
    pub k1: f32,
    pub k2: f32,
    pub k3: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LensProfile {
    pub maker: &'static str,
    pub model: &'static str,
    pub mounts: &'static [&'static str],
    pub geometry: LensGeometry,
    pub distortion: &'static [DistortionCalibration],
    pub tca: &'static [TcaCalibration],
    pub vignetting: &'static [VignettingCalibration],
}

impl LensProfile {
    #[must_use]
    pub fn distortion_at(self, focal: f32) -> Option<DistortionCalibration> {
        nearest_or_interpolated(self.distortion, focal, |calibration| match calibration {
            DistortionCalibration::Poly3 { focal, .. }
            | DistortionCalibration::PtLens { focal, .. } => *focal,
        })
    }

    #[must_use]
    pub fn tca_at(self, focal: f32) -> Option<TcaCalibration> {
        interpolate_tca(self.tca, focal)
    }

    #[must_use]
    pub fn vignetting_at(
        self,
        focal: f32,
        aperture: f32,
        distance: f32,
    ) -> Option<VignettingCalibration> {
        self.vignetting.iter().copied().min_by(|left, right| {
            vignetting_distance(*left, focal, aperture, distance)
                .total_cmp(&vignetting_distance(*right, focal, aperture, distance))
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LensfunSnapshot {
    pub commit: &'static str,
    pub timestamp: i64,
    pub cameras: &'static [CameraProfile],
    pub lenses: &'static [LensProfile],
}

impl LensfunSnapshot {
    #[must_use]
    pub const fn pinned() -> Self {
        Self {
            commit: LENSFUN_DATABASE_COMMIT,
            timestamp: LENSFUN_DATABASE_TIMESTAMP,
            cameras: &CAMERAS,
            lenses: &LENSES,
        }
    }

    #[must_use]
    pub fn find_camera(self, maker: &str, model: &str) -> Option<CameraProfile> {
        self.cameras.iter().copied().find(|camera| {
            same_name(camera.maker, maker)
                && (same_name(camera.model, model)
                    || camera.aliases.iter().any(|alias| same_name(alias, model)))
        })
    }

    #[must_use]
    pub fn find_lens(self, camera: Option<CameraProfile>, name: &str) -> Option<LensProfile> {
        let name = sanitize_lens_name(name);
        self.lenses.iter().copied().find(|lens| {
            same_name(lens.model, &name)
                && camera.is_none_or(|camera| {
                    lens.mounts
                        .iter()
                        .any(|mount| same_name(mount, camera.mount))
                })
        })
    }
}

const CAMERAS: [CameraProfile; 2] = [
    CameraProfile {
        maker: "Canon",
        model: "Canon EOS 5D Mark II",
        aliases: &["EOS 5D Mark II"],
        mount: "Canon EF",
        crop_factor: 1.0,
    },
    CameraProfile {
        maker: "Sony",
        model: "ILCE-7M3",
        aliases: &["Alpha 7 III"],
        mount: "Sony E",
        crop_factor: 1.0,
    },
];

const CANON_DISTORTION: [DistortionCalibration; 4] = [
    DistortionCalibration::PtLens {
        focal: 24.0,
        a: 0.015_298,
        b: -0.039_413,
        c: 0.0,
    },
    DistortionCalibration::PtLens {
        focal: 34.0,
        a: 0.006_539,
        b: -0.015_390,
        c: 0.0,
    },
    DistortionCalibration::PtLens {
        focal: 50.0,
        a: 0.0,
        b: 0.002,
        c: 0.0,
    },
    DistortionCalibration::PtLens {
        focal: 70.0,
        a: 0.0,
        b: 0.006_294,
        c: 0.0,
    },
];

const CANON_VIGNETTING: [VignettingCalibration; 2] = [
    VignettingCalibration {
        focal: 24.0,
        aperture: 2.8,
        distance: 10.0,
        k1: -0.9888,
        k2: -0.0305,
        k3: 0.2650,
    },
    VignettingCalibration {
        focal: 50.0,
        aperture: 2.8,
        distance: 10.0,
        k1: -0.7645,
        k2: 0.2124,
        k3: -0.0599,
    },
];

const SONY_DISTORTION: [DistortionCalibration; 4] = [
    DistortionCalibration::PtLens {
        focal: 24.0,
        a: 0.026_63,
        b: -0.093_53,
        c: 0.061_81,
    },
    DistortionCalibration::PtLens {
        focal: 34.0,
        a: 0.018_77,
        b: -0.052_29,
        c: 0.060_04,
    },
    DistortionCalibration::PtLens {
        focal: 50.0,
        a: 0.001_65,
        b: 0.011_34,
        c: 0.005_73,
    },
    DistortionCalibration::PtLens {
        focal: 105.0,
        a: 0.009_11,
        b: -0.016_75,
        c: 0.040_82,
    },
];

const SONY_TCA: [TcaCalibration; 3] = [
    TcaCalibration {
        focal: 24.0,
        red_linear: 1.000_274_6,
        red_cubic: -0.000_010_8,
        blue_linear: 1.000_133_1,
        blue_cubic: 0.0,
    },
    TcaCalibration {
        focal: 50.0,
        red_linear: 1.000_146_6,
        red_cubic: -0.000_019_4,
        blue_linear: 0.999_971_2,
        blue_cubic: 0.000_014_8,
    },
    TcaCalibration {
        focal: 105.0,
        red_linear: 0.999_896_4,
        red_cubic: -0.000_033_0,
        blue_linear: 0.999_904_5,
        blue_cubic: 0.000_000_1,
    },
];

const SONY_VIGNETTING: [VignettingCalibration; 2] = [
    VignettingCalibration {
        focal: 24.0,
        aperture: 4.0,
        distance: 10.0,
        k1: -0.230_405_1,
        k2: -0.511_525_9,
        k3: -0.088_333_5,
    },
    VignettingCalibration {
        focal: 50.0,
        aperture: 4.0,
        distance: 10.0,
        k1: -0.264_667_7,
        k2: -0.514_360_5,
        k3: 0.333_244_8,
    },
];

const LENSES: [LensProfile; 2] = [
    LensProfile {
        maker: "Canon",
        model: "Canon EF 24-70mm f/2.8L USM",
        mounts: &["Canon EF"],
        geometry: LensGeometry::Rectilinear,
        distortion: &CANON_DISTORTION,
        tca: &[],
        vignetting: &CANON_VIGNETTING,
    },
    LensProfile {
        maker: "Sony",
        model: "FE 24-105mm f/4 G OSS",
        mounts: &["Sony E"],
        geometry: LensGeometry::Rectilinear,
        distortion: &SONY_DISTORTION,
        tca: &SONY_TCA,
        vignetting: &SONY_VIGNETTING,
    },
];

fn same_name(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

fn sanitize_lens_name(name: &str) -> String {
    let end = name.find(" or ").into_iter().chain(name.find(" (")).min();
    name[..end.unwrap_or(name.len())].trim().to_owned()
}

fn nearest_or_interpolated<T: Copy>(
    values: &[T],
    target: f32,
    focal: impl Fn(&T) -> f32,
) -> Option<T> {
    values.iter().copied().min_by(|left, right| {
        (focal(left) - target)
            .abs()
            .total_cmp(&(focal(right) - target).abs())
    })
}

fn interpolate_tca(values: &[TcaCalibration], target: f32) -> Option<TcaCalibration> {
    let (left, right) = bracket(values, target)?;
    if (left.focal - right.focal).abs() < f32::EPSILON {
        return Some(left);
    }
    let amount = (target - left.focal) / (right.focal - left.focal);
    Some(TcaCalibration {
        focal: target,
        red_linear: lerp(left.red_linear, right.red_linear, amount),
        red_cubic: lerp(left.red_cubic, right.red_cubic, amount),
        blue_linear: lerp(left.blue_linear, right.blue_linear, amount),
        blue_cubic: lerp(left.blue_cubic, right.blue_cubic, amount),
    })
}

fn bracket(values: &[TcaCalibration], target: f32) -> Option<(TcaCalibration, TcaCalibration)> {
    let mut sorted = values.to_vec();
    sorted.sort_by(|left, right| left.focal.total_cmp(&right.focal));
    let first = *sorted.first()?;
    let last = *sorted.last()?;
    if target <= first.focal {
        return Some((first, first));
    }
    if target >= last.focal {
        return Some((last, last));
    }
    sorted
        .windows(2)
        .find_map(|pair| (target <= pair[1].focal).then_some((pair[0], pair[1])))
}

fn lerp(left: f32, right: f32, amount: f32) -> f32 {
    left + (right - left) * amount
}

fn vignetting_distance(
    value: VignettingCalibration,
    focal: f32,
    aperture: f32,
    distance: f32,
) -> f32 {
    (value.focal - focal).abs() * 4.0
        + (value.aperture - aperture).abs()
        + (value.distance - distance).abs() / distance.max(1.0)
}
