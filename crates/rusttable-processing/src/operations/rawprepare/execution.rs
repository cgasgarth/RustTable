//! Checked CPU planning and execution for the source `process` callback.
//!
//! Direct source lineage: `src/iop/rawprepare.c:194-447` and
//! `src/iop/rawprepare.c:568-739`; coupled metadata comes from
//! `src/common/image.h`, `src/common/image.c`, `src/common/dng_opcode.h`, and
//! `src/imageio/imageio_rawspeed.cc`.  This leaf accepts decoded samples and
//! metadata only. It does not decode camera files or call RawSpeed.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    clippy::manual_midpoint,
    clippy::missing_errors_doc,
    clippy::similar_names,
    clippy::too_many_arguments,
    reason = "the native callback is a source-shaped four-channel f32 operation"
)]

use std::fmt;

use rusttable_processing::RasterDimensions;

use super::codec::{RawPrepareFlatField, RawPrepareParametersV2};

pub const DT_IMAGE_RAW: u32 = 64;
pub const DT_IMAGE_HDR: u32 = 128;
pub const DT_IMAGE_S_RAW: u32 = 1 << 17;
pub const RAWPREPARE_CHANNELS: usize = 4;
pub const RAWPREPARE_DEFAULT_TILE_EDGE: u32 = 256;
pub const RAWPREPARE_MAX_CROP_PARAMETER: i32 = 65_535;
pub const RAWPREPARE_DEFAULT_MEMORY_BUDGET: usize = 512 * 1024 * 1024;
pub const RAWPREPARE_UNLIMITED_MEMORY: usize = usize::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RawPrepareSampleFormat {
    U16,
    F32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RawPrepareCfa {
    None,
    Bayer {
        /// Nonzero dcraw filter identity. The value is retained as metadata;
        /// phase is the operation-local equivalent of
        /// `dt_rawspeed_crop_dcraw_filters`.
        filters: u32,
        phase_x: u8,
        phase_y: u8,
    },
    XTrans {
        pattern: [[u8; 6]; 6],
        phase_x: u8,
        phase_y: u8,
    },
}

impl RawPrepareCfa {
    #[must_use]
    pub const fn bayer(filters: u32, phase_x: u8, phase_y: u8) -> Self {
        Self::Bayer {
            filters,
            phase_x: phase_x % 2,
            phase_y: phase_y % 2,
        }
    }

    #[must_use]
    pub const fn xtrans(pattern: [[u8; 6]; 6], phase_x: u8, phase_y: u8) -> Self {
        Self::XTrans {
            pattern,
            phase_x: phase_x % 6,
            phase_y: phase_y % 6,
        }
    }

    #[must_use]
    pub const fn is_mosaic(self) -> bool {
        matches!(self, Self::Bayer { .. } | Self::XTrans { .. })
    }

    #[must_use]
    pub const fn after_crop(self, left: u32, top: u32) -> Self {
        match self {
            Self::None => Self::None,
            Self::Bayer {
                filters,
                phase_x,
                phase_y,
            } => Self::Bayer {
                filters,
                phase_x: ((phase_x as u32 + left) % 2) as u8,
                phase_y: ((phase_y as u32 + top) % 2) as u8,
            },
            Self::XTrans {
                pattern,
                phase_x,
                phase_y,
            } => Self::XTrans {
                pattern,
                phase_x: ((phase_x as u32 + left) % 6) as u8,
                phase_y: ((phase_y as u32 + top) % 6) as u8,
            },
        }
    }

    /// Returns the X-Trans table as the native `_adjust_xtrans_filters`
    /// callback publishes it after the active-area crop.
    #[must_use]
    pub const fn xtrans_table_after_crop(self, left: u32, top: u32) -> Option<[[u8; 6]; 6]> {
        let Self::XTrans { pattern, .. } = self else {
            return None;
        };
        let mut output = [[0; 6]; 6];
        let mut y = 0;
        while y < 6 {
            let mut x = 0;
            while x < 6 {
                output[y][x] = pattern[(y + top as usize) % 6][(x + left as usize) % 6];
                x += 1;
            }
            y += 1;
        }
        Some(output)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RawPrepareCrop {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl RawPrepareCrop {
    #[must_use]
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    #[must_use]
    pub const fn left(self) -> i32 {
        self.left
    }

    #[must_use]
    pub const fn top(self) -> i32 {
        self.top
    }

    #[must_use]
    pub const fn right(self) -> i32 {
        self.right
    }

    #[must_use]
    pub const fn bottom(self) -> i32 {
        self.bottom
    }
}

/// The subset of `dt_image_t` required by `rawprepare.c`.  Camera decoding and
/// metadata acquisition remain outside this operation-local boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct RawPrepareImageMetadata {
    dimensions: RasterDimensions,
    flags: u32,
    sample_format: RawPrepareSampleFormat,
    channels: u8,
    cfa: RawPrepareCfa,
    crop: RawPrepareCrop,
    raw_black_level_separate: [u16; 4],
    raw_white_point: u32,
    gain_maps: Vec<RawPrepareGainMap>,
}

impl RawPrepareImageMetadata {
    #[must_use]
    pub fn new(
        dimensions: RasterDimensions,
        flags: u32,
        sample_format: RawPrepareSampleFormat,
        channels: u8,
        cfa: RawPrepareCfa,
        crop: RawPrepareCrop,
        raw_black_level_separate: [u16; 4],
        raw_white_point: u32,
    ) -> Self {
        Self {
            dimensions,
            flags,
            sample_format,
            channels,
            cfa,
            crop,
            raw_black_level_separate,
            raw_white_point,
            gain_maps: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_gain_maps(mut self, gain_maps: Vec<RawPrepareGainMap>) -> Self {
        self.gain_maps = gain_maps;
        self
    }

    #[must_use]
    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    #[must_use]
    pub const fn sample_format(&self) -> RawPrepareSampleFormat {
        self.sample_format
    }

    #[must_use]
    pub const fn channels(&self) -> u8 {
        self.channels
    }

    #[must_use]
    pub const fn cfa(&self) -> RawPrepareCfa {
        self.cfa
    }

    #[must_use]
    pub const fn crop(&self) -> RawPrepareCrop {
        self.crop
    }

    #[must_use]
    pub const fn raw_black_level_separate(&self) -> [u16; 4] {
        self.raw_black_level_separate
    }

    #[must_use]
    pub const fn raw_white_point(&self) -> u32 {
        self.raw_white_point
    }

    #[must_use]
    pub fn gain_maps(&self) -> &[RawPrepareGainMap] {
        &self.gain_maps
    }

    /// Mirrors `reload_defaults`: image-loader crop and level metadata are
    /// copied into the current v2 parameter declaration. Embedded gain maps
    /// are selected by default only when all four maps pass the native shape
    /// checks and fit into the v2 u16 white-point field.
    pub fn default_parameters(&self) -> Result<RawPrepareParametersV2, RawPrepareError> {
        let raw_white_point = u16::try_from(self.raw_white_point)
            .map_err(|_| RawPrepareError::MetadataOutOfRange("raw white point"))?;
        let flat_field = if RawPrepareGainMapSet::try_new(self.dimensions, &self.gain_maps).is_ok()
        {
            RawPrepareFlatField::Embedded
        } else {
            RawPrepareFlatField::Off
        };
        Ok(RawPrepareParametersV2::new(
            self.crop.left,
            self.crop.top,
            self.crop.right,
            self.crop.bottom,
            self.raw_black_level_separate,
            raw_white_point,
            flat_field,
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RawPrepareMemoryBudget {
    maximum_bytes: usize,
}

impl RawPrepareMemoryBudget {
    #[must_use]
    pub const fn new(maximum_bytes: usize) -> Self {
        Self { maximum_bytes }
    }

    #[must_use]
    pub const fn maximum_bytes(self) -> usize {
        self.maximum_bytes
    }
}

impl Default for RawPrepareMemoryBudget {
    fn default() -> Self {
        Self::new(RAWPREPARE_DEFAULT_MEMORY_BUDGET)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RawPrepareTiling {
    pub overlap_pixels: u32,
    pub alignment_pixels: u32,
    pub minimum_tile_edge: u32,
    pub preferred_tile_edge: u32,
    pub input_multiplier_milli: u32,
    pub output_multiplier_milli: u32,
    pub temporary_multiplier_milli: u32,
}

impl Default for RawPrepareTiling {
    fn default() -> Self {
        Self {
            overlap_pixels: 0,
            alignment_pixels: 1,
            minimum_tile_edge: 1,
            preferred_tile_edge: RAWPREPARE_DEFAULT_TILE_EDGE,
            input_multiplier_milli: 1000,
            output_multiplier_milli: 1000,
            temporary_multiplier_milli: 1000,
        }
    }
}

/// The source `modify_roi_*` contract for one output tile. `input` is the
/// row-major buffer supplied by the upstream operation; `output` uses the
/// global output coordinates used by `_BL` and gain-map interpolation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawPrepareTile {
    input: RasterDimensions,
    output_x: u32,
    output_y: u32,
    output: RasterDimensions,
    crop_x: u32,
    crop_y: u32,
}

impl RawPrepareTile {
    pub fn new(
        input: RasterDimensions,
        output_x: u32,
        output_y: u32,
        output: RasterDimensions,
        roi_in_scale: f32,
        piece_scale: f32,
        crop: RawPrepareCrop,
    ) -> Result<Self, RawPrepareError> {
        let crop_x = scaled_crop(crop.left, roi_in_scale, piece_scale)?;
        let crop_y = scaled_crop(crop.top, roi_in_scale, piece_scale)?;
        let required_width = crop_x
            .checked_add(output.width())
            .ok_or(RawPrepareError::RoiArithmeticOverflow)?;
        let required_height = crop_y
            .checked_add(output.height())
            .ok_or(RawPrepareError::RoiArithmeticOverflow)?;
        if required_width > input.width() || required_height > input.height() {
            return Err(RawPrepareError::InputRoiTooSmall);
        }
        Ok(Self {
            input,
            output_x,
            output_y,
            output,
            crop_x,
            crop_y,
        })
    }

    #[must_use]
    pub const fn input(&self) -> RasterDimensions {
        self.input
    }

    #[must_use]
    pub const fn output(&self) -> RasterDimensions {
        self.output
    }

    #[must_use]
    pub const fn output_x(&self) -> u32 {
        self.output_x
    }

    #[must_use]
    pub const fn output_y(&self) -> u32 {
        self.output_y
    }

    #[must_use]
    pub const fn crop_x(&self) -> u32 {
        self.crop_x
    }

    #[must_use]
    pub const fn crop_y(&self) -> u32 {
        self.crop_y
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawPrepareAlphaBehavior {
    /// The native CPU `rawprepare_4f` path changes all `piece->colors`
    /// channels, including the fourth lane. It is not silently treated as
    /// premultiplied or preserved alpha.
    CpuNormalizesFourthChannel,
    NotApplicableToSinglePlane,
    /// The retained OpenCL kernel preserves `.w`; GPU routing is deferred and
    /// therefore is not exposed as a capability by this leaf.
    GpuFourthChannelPreservedDeferred,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawPrepareGainMap {
    top: u32,
    left: u32,
    bottom: u32,
    right: u32,
    plane: u32,
    planes: u32,
    row_pitch: u32,
    col_pitch: u32,
    map_points_v: u32,
    map_points_h: u32,
    map_spacing_v: f64,
    map_spacing_h: f64,
    map_origin_v: f64,
    map_origin_h: f64,
    map_planes: u32,
    map_gain: Vec<f32>,
}

impl RawPrepareGainMap {
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        top: u32,
        left: u32,
        bottom: u32,
        right: u32,
        map_points_v: u32,
        map_points_h: u32,
        map_spacing_v: f64,
        map_spacing_h: f64,
        map_origin_v: f64,
        map_origin_h: f64,
        map_gain: Vec<f32>,
    ) -> Self {
        Self {
            top,
            left,
            bottom,
            right,
            plane: 0,
            planes: 1,
            row_pitch: 2,
            col_pitch: 2,
            map_points_v,
            map_points_h,
            map_spacing_v,
            map_spacing_h,
            map_origin_v,
            map_origin_h,
            map_planes: 1,
            map_gain,
        }
    }

    #[must_use]
    pub fn map_gain(&self) -> &[f32] {
        &self.map_gain
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawPrepareGainMapSet {
    maps: [RawPrepareGainMap; 4],
}

impl RawPrepareGainMapSet {
    /// Mirrors `_check_gain_maps`: exactly four Bayer planes, full-image
    /// coverage, 2-pixel CFA spacing, and a common interpolation grid.
    pub fn try_new(
        dimensions: RasterDimensions,
        maps: &[RawPrepareGainMap],
    ) -> Result<Self, RawPrepareError> {
        if maps.len() != 4 {
            return Err(RawPrepareError::UnsupportedGainMaps);
        }
        let mut by_filter: [Option<RawPrepareGainMap>; 4] = [None, None, None, None];
        for map in maps {
            let expected_len = usize::try_from(map.map_points_h)
                .ok()
                .and_then(|width| {
                    usize::try_from(map.map_points_v)
                        .ok()
                        .and_then(|height| width.checked_mul(height))
                })
                .ok_or(RawPrepareError::GainMapArithmeticOverflow)?;
            if map.plane != 0
                || map.planes != 1
                || map.map_planes != 1
                || map.row_pitch != 2
                || map.col_pitch != 2
                || map.map_points_v < 2
                || map.map_points_h < 2
                || map.top > 1
                || map.left > 1
                || map.bottom != dimensions.height()
                || map.right != dimensions.width()
                || map.map_gain.len() != expected_len
                || !map.map_spacing_v.is_finite()
                || !map.map_spacing_h.is_finite()
                || map.map_spacing_v == 0.0
                || map.map_spacing_h == 0.0
                || !map.map_origin_v.is_finite()
                || !map.map_origin_h.is_finite()
                || map.map_gain.iter().any(|gain| !gain.is_finite())
            {
                return Err(RawPrepareError::UnsupportedGainMaps);
            }
            let filter = (((map.top & 1) << 1) + (map.left & 1)) as usize;
            if by_filter[filter].replace(map.clone()).is_some() {
                return Err(RawPrepareError::UnsupportedGainMaps);
            }
        }
        let maps = by_filter
            .map(|map| map.ok_or(RawPrepareError::UnsupportedGainMaps))
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;
        let maps: [RawPrepareGainMap; 4] = maps
            .try_into()
            .map_err(|_| RawPrepareError::UnsupportedGainMaps)?;
        for map in &maps[1..] {
            if map.map_points_h != maps[0].map_points_h
                || map.map_points_v != maps[0].map_points_v
                || map.map_spacing_h != maps[0].map_spacing_h
                || map.map_spacing_v != maps[0].map_spacing_v
                || map.map_origin_h != maps[0].map_origin_h
                || map.map_origin_v != maps[0].map_origin_v
            {
                return Err(RawPrepareError::UnsupportedGainMaps);
            }
        }
        Ok(Self { maps })
    }

    #[must_use]
    pub fn maps(&self) -> &[RawPrepareGainMap; 4] {
        &self.maps
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawPrepareInputKind {
    MosaicU16,
    MosaicF32,
    FourChannelF32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawPreparePlan {
    dimensions: RasterDimensions,
    output_dimensions: RasterDimensions,
    crop: RawPrepareCrop,
    input_kind: RawPrepareInputKind,
    cfa: RawPrepareCfa,
    output_cfa: RawPrepareCfa,
    sub: [f32; 4],
    div: [f32; 4],
    raw_black_level: u16,
    raw_white_point: u16,
    gain_maps: Option<RawPrepareGainMapSet>,
    memory_budget: RawPrepareMemoryBudget,
    tiling: RawPrepareTiling,
}

impl RawPreparePlan {
    pub fn new(
        metadata: &RawPrepareImageMetadata,
        parameters: &RawPrepareParametersV2,
    ) -> Result<Self, RawPrepareError> {
        Self::new_with_budget(metadata, parameters, RawPrepareMemoryBudget::default())
    }

    pub fn new_with_budget(
        metadata: &RawPrepareImageMetadata,
        parameters: &RawPrepareParametersV2,
        memory_budget: RawPrepareMemoryBudget,
    ) -> Result<Self, RawPrepareError> {
        if metadata.flags & (DT_IMAGE_RAW | DT_IMAGE_S_RAW) == 0 {
            return Err(RawPrepareError::UnsupportedCamera);
        }
        let input_kind = classify_input(metadata)?;
        let crop = RawPrepareCrop::new(
            parameters.left(),
            parameters.top(),
            parameters.right(),
            parameters.bottom(),
        );
        let output_dimensions = cropped_dimensions(metadata.dimensions, crop)?;
        let cfa = metadata.cfa;
        let output_cfa = cfa.after_crop(crop.left as u32, crop.top as u32);
        let raw_white_point = parameters.raw_white_point();
        let (sub, div) = if input_kind == RawPrepareInputKind::FourChannelF32 {
            let normalizer = if metadata.flags & DT_IMAGE_HDR == DT_IMAGE_HDR {
                1.0
            } else {
                f32::from(u16::MAX)
            };
            let white = f32::from(raw_white_point) / normalizer;
            let sub = parameters
                .raw_black_level_separate()
                .map(|level| f32::from(level) / normalizer);
            let div = sub.map(|value| white - value);
            (sub, div)
        } else {
            let white = f32::from(raw_white_point);
            let sub = parameters.raw_black_level_separate().map(f32::from);
            let div = sub.map(|value| white - value);
            (sub, div)
        };
        if div.iter().any(|value| *value <= 0.0 || !value.is_finite()) {
            return Err(RawPrepareError::InvalidBlackWhiteLevels);
        }
        let black_sum: u32 = parameters
            .raw_black_level_separate()
            .into_iter()
            .map(u32::from)
            .sum();
        let raw_black_level = (black_sum as f32 / 4.0).round() as u16;
        let gain_maps = if parameters.flat_field() == RawPrepareFlatField::Embedded
            && matches!(cfa, RawPrepareCfa::Bayer { .. })
        {
            RawPrepareGainMapSet::try_new(metadata.dimensions, metadata.gain_maps()).ok()
        } else {
            None
        };
        let plan = Self {
            dimensions: metadata.dimensions,
            output_dimensions,
            crop,
            input_kind,
            cfa,
            output_cfa,
            sub,
            div,
            raw_black_level,
            raw_white_point,
            gain_maps,
            memory_budget,
            tiling: RawPrepareTiling::default(),
        };
        Ok(plan)
    }

    #[must_use]
    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn output_dimensions(&self) -> RasterDimensions {
        self.output_dimensions
    }

    #[must_use]
    pub const fn crop(&self) -> RawPrepareCrop {
        self.crop
    }

    #[must_use]
    pub const fn input_kind(&self) -> RawPrepareInputKind {
        self.input_kind
    }

    #[must_use]
    pub const fn cfa(&self) -> RawPrepareCfa {
        self.cfa
    }

    #[must_use]
    pub const fn output_cfa(&self) -> RawPrepareCfa {
        self.output_cfa
    }

    #[must_use]
    pub const fn raw_black_level(&self) -> u16 {
        self.raw_black_level
    }

    #[must_use]
    pub const fn raw_white_point(&self) -> u16 {
        self.raw_white_point
    }

    /// Mirrors the native `process` publication of a unit processed maximum
    /// for all four buffer lanes.
    #[must_use]
    pub const fn processed_maximum() -> [f32; 4] {
        [1.0; 4]
    }

    #[must_use]
    pub const fn tiling(&self) -> RawPrepareTiling {
        self.tiling
    }

    #[must_use]
    pub const fn alpha_behavior(&self) -> RawPrepareAlphaBehavior {
        match self.input_kind {
            RawPrepareInputKind::FourChannelF32 => {
                RawPrepareAlphaBehavior::CpuNormalizesFourthChannel
            }
            RawPrepareInputKind::MosaicU16 | RawPrepareInputKind::MosaicF32 => {
                RawPrepareAlphaBehavior::NotApplicableToSinglePlane
            }
        }
    }

    #[must_use]
    pub const fn gpu_alpha_behavior() -> RawPrepareAlphaBehavior {
        RawPrepareAlphaBehavior::GpuFourthChannelPreservedDeferred
    }

    pub fn full_frame_tile(&self) -> Result<RawPrepareTile, RawPrepareError> {
        RawPrepareTile::new(
            self.dimensions,
            0,
            0,
            self.output_dimensions,
            1.0,
            1.0,
            self.crop,
        )
    }

    pub fn execute_u16<F: Fn() -> bool>(
        &self,
        input: &[u16],
        tile: RawPrepareTile,
        cancelled: F,
    ) -> Result<Vec<f32>, RawPrepareError> {
        if self.input_kind != RawPrepareInputKind::MosaicU16 {
            return Err(RawPrepareError::InputKindMismatch);
        }
        self.execute_mosaic(input.len(), tile, cancelled, |index| {
            f32::from(input[index])
        })
    }

    pub fn execute_f32<F: Fn() -> bool>(
        &self,
        input: &[f32],
        tile: RawPrepareTile,
        cancelled: F,
    ) -> Result<Vec<f32>, RawPrepareError> {
        if self.input_kind != RawPrepareInputKind::MosaicF32 {
            return Err(RawPrepareError::InputKindMismatch);
        }
        self.execute_mosaic(input.len(), tile, cancelled, |index| input[index])
    }

    pub fn execute_four_channel<F: Fn() -> bool>(
        &self,
        input: &[[f32; 4]],
        tile: RawPrepareTile,
        cancelled: F,
    ) -> Result<Vec<[f32; 4]>, RawPrepareError> {
        if self.input_kind != RawPrepareInputKind::FourChannelF32 {
            return Err(RawPrepareError::InputKindMismatch);
        }
        self.validate_tile_input(input.len(), tile)?;
        let output_count = pixel_count(tile.output);
        self.check_memory(output_count, std::mem::size_of::<[f32; 4]>())?;
        let mut output = allocate::<[f32; 4]>(output_count)?;
        let input_width = usize::try_from(tile.input.width())
            .map_err(|_| RawPrepareError::RoiArithmeticOverflow)?;
        let output_width = usize::try_from(tile.output.width())
            .map_err(|_| RawPrepareError::RoiArithmeticOverflow)?;
        for y in 0..tile.output.height() {
            if cancelled() {
                return Err(RawPrepareError::Cancelled);
            }
            for x in 0..tile.output.width() {
                let in_index = usize::try_from(y + tile.crop_y)
                    .ok()
                    .and_then(|row| row.checked_mul(input_width))
                    .and_then(|index| {
                        usize::try_from(x + tile.crop_x)
                            .ok()
                            .and_then(|col| index.checked_add(col))
                    })
                    .ok_or(RawPrepareError::RoiArithmeticOverflow)?;
                let out_index = usize::try_from(y)
                    .ok()
                    .and_then(|row| row.checked_mul(output_width))
                    .and_then(|index| {
                        usize::try_from(x)
                            .ok()
                            .and_then(|col| index.checked_add(col))
                    })
                    .ok_or(RawPrepareError::RoiArithmeticOverflow)?;
                let pixel = input
                    .get(in_index)
                    .ok_or(RawPrepareError::InputRoiTooSmall)?;
                let mut normalized = [0.0; 4];
                for channel in 0..RAWPREPARE_CHANNELS {
                    let value = pixel[channel];
                    if !value.is_finite() {
                        return Err(RawPrepareError::NonFiniteInput { index: in_index });
                    }
                    normalized[channel] = (value - self.sub[channel]) / self.div[channel];
                }
                output[out_index] = normalized;
            }
        }
        Ok(output)
    }

    fn execute_mosaic<F, C>(
        &self,
        input_len: usize,
        tile: RawPrepareTile,
        cancelled: C,
        read: F,
    ) -> Result<Vec<f32>, RawPrepareError>
    where
        F: Fn(usize) -> f32,
        C: Fn() -> bool,
    {
        self.validate_tile_input(input_len, tile)?;
        let output_count = pixel_count(tile.output);
        self.check_memory(output_count, std::mem::size_of::<f32>())?;
        let mut output = allocate::<f32>(output_count)?;
        let input_width = usize::try_from(tile.input.width())
            .map_err(|_| RawPrepareError::RoiArithmeticOverflow)?;
        let output_width = usize::try_from(tile.output.width())
            .map_err(|_| RawPrepareError::RoiArithmeticOverflow)?;
        for y in 0..tile.output.height() {
            if cancelled() {
                return Err(RawPrepareError::Cancelled);
            }
            for x in 0..tile.output.width() {
                let in_index = usize::try_from(y + tile.crop_y)
                    .ok()
                    .and_then(|row| row.checked_mul(input_width))
                    .and_then(|index| {
                        usize::try_from(x + tile.crop_x)
                            .ok()
                            .and_then(|col| index.checked_add(col))
                    })
                    .ok_or(RawPrepareError::RoiArithmeticOverflow)?;
                let out_index = usize::try_from(y)
                    .ok()
                    .and_then(|row| row.checked_mul(output_width))
                    .and_then(|index| {
                        usize::try_from(x)
                            .ok()
                            .and_then(|col| index.checked_add(col))
                    })
                    .ok_or(RawPrepareError::RoiArithmeticOverflow)?;
                let value = read(in_index);
                if !value.is_finite() {
                    return Err(RawPrepareError::NonFiniteInput { index: in_index });
                }
                let filter = black_level_index(&self.crop, tile, x, y);
                let mut value = (value - self.sub[filter]) / self.div[filter];
                if let Some(gain_maps) = &self.gain_maps {
                    value *= gain_map_gain(gain_maps, tile, x, y, self.dimensions, filter)?;
                }
                if !value.is_finite() {
                    return Err(RawPrepareError::NonFiniteOutput { index: out_index });
                }
                output[out_index] = value;
            }
        }
        Ok(output)
    }

    fn validate_tile_input(
        &self,
        input_len: usize,
        tile: RawPrepareTile,
    ) -> Result<(), RawPrepareError> {
        if tile.output_x >= self.output_dimensions.width()
            || tile.output_y >= self.output_dimensions.height()
            || tile
                .output_x
                .checked_add(tile.output.width())
                .ok_or(RawPrepareError::RoiArithmeticOverflow)?
                > self.output_dimensions.width()
            || tile
                .output_y
                .checked_add(tile.output.height())
                .ok_or(RawPrepareError::RoiArithmeticOverflow)?
                > self.output_dimensions.height()
        {
            return Err(RawPrepareError::OutputRoiOutOfBounds);
        }
        let expected = pixel_count(tile.input);
        let expected_usize =
            usize::try_from(expected).map_err(|_| RawPrepareError::RoiArithmeticOverflow)?;
        if input_len != expected_usize {
            return Err(RawPrepareError::InputLengthMismatch {
                expected,
                actual: input_len,
            });
        }
        Ok(())
    }

    fn check_memory(&self, pixels: u64, bytes_per_pixel: usize) -> Result<(), RawPrepareError> {
        let pixels = usize::try_from(pixels).map_err(|_| RawPrepareError::RoiArithmeticOverflow)?;
        let required = pixels
            .checked_mul(bytes_per_pixel)
            .ok_or(RawPrepareError::RoiArithmeticOverflow)?;
        if required > self.memory_budget.maximum_bytes {
            return Err(RawPrepareError::MemoryBudgetExceeded {
                required,
                budget: self.memory_budget.maximum_bytes,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawPrepareError {
    UnsupportedCamera,
    UnsupportedLayout,
    AlreadyNormalized,
    MetadataOutOfRange(&'static str),
    InvalidCrop,
    InvalidBlackWhiteLevels,
    UnsupportedGainMaps,
    GainMapArithmeticOverflow,
    RoiArithmeticOverflow,
    InputRoiTooSmall,
    OutputRoiOutOfBounds,
    InputLengthMismatch { expected: u64, actual: usize },
    InputKindMismatch,
    MemoryBudgetExceeded { required: usize, budget: usize },
    AllocationFailed { required: usize },
    NonFiniteInput { index: usize },
    NonFiniteOutput { index: usize },
    Cancelled,
}

impl fmt::Display for RawPrepareError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedCamera => formatter.write_str("rawprepare camera is unsupported"),
            Self::UnsupportedLayout => {
                formatter.write_str("rawprepare input layout is unsupported")
            }
            Self::AlreadyNormalized => {
                formatter.write_str("rawprepare input is already normalized")
            }
            Self::MetadataOutOfRange(field) => {
                write!(formatter, "rawprepare {field} is out of range")
            }
            Self::InvalidCrop => formatter.write_str("rawprepare crop parameters are invalid"),
            Self::InvalidBlackWhiteLevels => {
                formatter.write_str("rawprepare white point is not above every black level")
            }
            Self::UnsupportedGainMaps => {
                formatter.write_str("rawprepare embedded gain maps are unsupported")
            }
            Self::GainMapArithmeticOverflow => {
                formatter.write_str("rawprepare gain-map dimensions overflowed")
            }
            Self::RoiArithmeticOverflow => {
                formatter.write_str("rawprepare ROI arithmetic overflowed")
            }
            Self::InputRoiTooSmall => formatter.write_str("rawprepare input ROI is too small"),
            Self::OutputRoiOutOfBounds => {
                formatter.write_str("rawprepare output ROI is out of bounds")
            }
            Self::InputLengthMismatch { expected, actual } => write!(
                formatter,
                "rawprepare input has {actual} samples; expected {expected}"
            ),
            Self::InputKindMismatch => {
                formatter.write_str("rawprepare input kind does not match the plan")
            }
            Self::MemoryBudgetExceeded { required, budget } => write!(
                formatter,
                "rawprepare needs {required} bytes; budget is {budget}"
            ),
            Self::AllocationFailed { required } => {
                write!(formatter, "rawprepare could not allocate {required} bytes")
            }
            Self::NonFiniteInput { index } => {
                write!(formatter, "rawprepare input sample {index} is non-finite")
            }
            Self::NonFiniteOutput { index } => {
                write!(formatter, "rawprepare output sample {index} is non-finite")
            }
            Self::Cancelled => formatter.write_str("rawprepare execution was cancelled"),
        }
    }
}

impl std::error::Error for RawPrepareError {}

fn classify_input(
    metadata: &RawPrepareImageMetadata,
) -> Result<RawPrepareInputKind, RawPrepareError> {
    if let RawPrepareCfa::Bayer { filters, .. } = metadata.cfa
        && (filters == 0 || filters == 9)
    {
        return Err(RawPrepareError::UnsupportedCamera);
    }
    if metadata.channels == 1 && metadata.cfa.is_mosaic() {
        if metadata.sample_format == RawPrepareSampleFormat::F32
            && (metadata.flags & DT_IMAGE_HDR == 0
                || metadata.raw_white_point == 1
                || metadata.raw_white_point == 0x3F80_0000)
        {
            return Err(RawPrepareError::AlreadyNormalized);
        }
        return Ok(match metadata.sample_format {
            RawPrepareSampleFormat::U16 => RawPrepareInputKind::MosaicU16,
            RawPrepareSampleFormat::F32 => RawPrepareInputKind::MosaicF32,
        });
    }
    if metadata.channels == RAWPREPARE_CHANNELS as u8
        && metadata.sample_format == RawPrepareSampleFormat::F32
        && metadata.cfa == RawPrepareCfa::None
    {
        return Ok(RawPrepareInputKind::FourChannelF32);
    }
    Err(if metadata.cfa.is_mosaic() {
        RawPrepareError::UnsupportedLayout
    } else {
        RawPrepareError::UnsupportedCamera
    })
}

fn cropped_dimensions(
    dimensions: RasterDimensions,
    crop: RawPrepareCrop,
) -> Result<RasterDimensions, RawPrepareError> {
    if crop.left < 0
        || crop.top < 0
        || crop.right < 0
        || crop.bottom < 0
        || crop.left > RAWPREPARE_MAX_CROP_PARAMETER
        || crop.top > RAWPREPARE_MAX_CROP_PARAMETER
        || crop.right > RAWPREPARE_MAX_CROP_PARAMETER
        || crop.bottom > RAWPREPARE_MAX_CROP_PARAMETER
    {
        return Err(RawPrepareError::InvalidCrop);
    }
    let horizontal = i64::from(crop.left) + i64::from(crop.right);
    let vertical = i64::from(crop.top) + i64::from(crop.bottom);
    // `_image_set_rawcrops` deliberately requires less than half the source
    // dimension, not merely a positive result.
    if horizontal >= i64::from(dimensions.width() / 2)
        || vertical >= i64::from(dimensions.height() / 2)
    {
        return Err(RawPrepareError::InvalidCrop);
    }
    let width = u32::try_from(i64::from(dimensions.width()) - horizontal)
        .map_err(|_| RawPrepareError::InvalidCrop)?;
    let height = u32::try_from(i64::from(dimensions.height()) - vertical)
        .map_err(|_| RawPrepareError::InvalidCrop)?;
    RasterDimensions::new(width, height).map_err(|_| RawPrepareError::InvalidCrop)
}

fn scaled_crop(crop: i32, roi_scale: f32, piece_scale: f32) -> Result<u32, RawPrepareError> {
    if crop < 0 || !roi_scale.is_finite() || !piece_scale.is_finite() || piece_scale <= 0.0 {
        return Err(RawPrepareError::InvalidCrop);
    }
    let scaled = (crop as f32 * roi_scale / piece_scale).round();
    if !scaled.is_finite() || scaled < 0.0 || scaled > u32::MAX as f32 {
        return Err(RawPrepareError::RoiArithmeticOverflow);
    }
    Ok(scaled as u32)
}

fn pixel_count(dimensions: RasterDimensions) -> u64 {
    dimensions.pixel_count()
}

fn allocate<T: Default + Clone>(count: u64) -> Result<Vec<T>, RawPrepareError> {
    let count = usize::try_from(count).map_err(|_| RawPrepareError::RoiArithmeticOverflow)?;
    let required = count.saturating_mul(std::mem::size_of::<T>());
    let mut output = Vec::new();
    output
        .try_reserve_exact(count)
        .map_err(|_| RawPrepareError::AllocationFailed { required })?;
    output.resize(count, T::default());
    Ok(output)
}

fn black_level_index(crop: &RawPrepareCrop, tile: RawPrepareTile, x: u32, y: u32) -> usize {
    let row = (y + tile.output_y + crop.top as u32) & 1;
    let col = (x + tile.output_x + crop.left as u32) & 1;
    ((row << 1) + col) as usize
}

fn gain_map_gain(
    maps: &RawPrepareGainMapSet,
    tile: RawPrepareTile,
    x: u32,
    y: u32,
    dimensions: RasterDimensions,
    filter: usize,
) -> Result<f32, RawPrepareError> {
    let map = maps
        .maps
        .get(filter)
        .ok_or(RawPrepareError::UnsupportedGainMaps)?;
    let map_width = map.map_points_h;
    let map_height = map.map_points_v;
    let image_to_rel_x = 1.0 / f64::from(dimensions.width());
    let image_to_rel_y = 1.0 / f64::from(dimensions.height());
    let rel_to_map_x = 1.0 / map.map_spacing_h;
    let rel_to_map_y = 1.0 / map.map_spacing_v;
    let map_x = (((f64::from(tile.output_x + tile.crop_x + x) * image_to_rel_x)
        - map.map_origin_h)
        * rel_to_map_x)
        .clamp(0.0, f64::from(map_width));
    let map_y = (((f64::from(tile.output_y + tile.crop_y + y) * image_to_rel_y)
        - map.map_origin_v)
        * rel_to_map_y)
        .clamp(0.0, f64::from(map_height));
    let x0 = (map_x as u32).min(map_width - 1);
    let x1 = x0.saturating_add(1).min(map_width - 1);
    let y0 = (map_y as u32).min(map_height - 1);
    let y1 = y0.saturating_add(1).min(map_height - 1);
    let x_fraction = map_x - f64::from(x0);
    let y_fraction = map_y - f64::from(y0);
    let row = |row: u32, col: u32| -> Result<f32, RawPrepareError> {
        let index = usize::try_from(row)
            .ok()
            .and_then(|row| {
                usize::try_from(map_width)
                    .ok()
                    .and_then(|width| row.checked_mul(width))
            })
            .and_then(|index| {
                usize::try_from(col)
                    .ok()
                    .and_then(|col| index.checked_add(col))
            })
            .ok_or(RawPrepareError::GainMapArithmeticOverflow)?;
        map.map_gain
            .get(index)
            .copied()
            .ok_or(RawPrepareError::UnsupportedGainMaps)
    };
    let top = f64::from(row(y0, x0)?) * (1.0 - x_fraction) + f64::from(row(y0, x1)?) * x_fraction;
    let bottom =
        f64::from(row(y1, x0)?) * (1.0 - x_fraction) + f64::from(row(y1, x1)?) * x_fraction;
    Ok((top * (1.0 - y_fraction) + bottom * y_fraction) as f32)
}
