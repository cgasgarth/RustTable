use super::codec::{RasterFileChannelMode, RasterMaskAsset, RasterMaskFormat};
use rusttable_image::Roi;
use rusttable_masks::{
    MaskIdentity, MaskRoi, ProducerIdentity, RasterMaskDescriptor, RasterMaskPublication,
};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::fmt;

/// Canonical mask publication metadata used by the CPU and GPU paths.
pub const RASTERFILE_WGSL: &str = r"
struct Params {
    width: u32, height: u32, src_stride: u32, dst_stride: u32,
    src_x: u32, src_y: u32, dst_x: u32, dst_y: u32,
};
@group(0) @binding(0) var<storage, read> source: array<f32>;
@group(0) @binding(1) var<storage, read_write> destination: array<f32>;
@group(0) @binding(2) var<uniform> params: Params;
@compute @workgroup_size(64)
fn copy_mask(@builtin(global_invocation_id) id: vec3<u32>) {
    if (params.width == 0u || params.height == 0u) { return; }
    let count = params.width * params.height;
    if (id.x >= count) { return; }
    let x = id.x % params.width;
    let y = id.x / params.width;
    let source_index = (params.src_y + y) * params.src_stride + params.src_x + x;
    let destination_index = (params.dst_y + y) * params.dst_stride + params.dst_x + x;
    destination[destination_index] = clamp(source[source_index], 0.0, 1.0);
}
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasterFileReceipt {
    identity: [u8; 32],
    format: RasterMaskFormat,
    dimensions: (u32, u32),
    mode: RasterFileChannelMode,
    mask_hash: [u8; 32],
}

impl RasterFileReceipt {
    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }
    #[must_use]
    pub const fn format(&self) -> RasterMaskFormat {
        self.format
    }
    #[must_use]
    pub const fn dimensions(&self) -> (u32, u32) {
        self.dimensions
    }
    #[must_use]
    pub const fn mode(&self) -> RasterFileChannelMode {
        self.mode
    }
    #[must_use]
    pub const fn mask_hash(&self) -> [u8; 32] {
        self.mask_hash
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RasterFilePlan {
    asset: RasterMaskAsset,
    descriptor: RasterMaskDescriptor,
    receipt: RasterFileReceipt,
}

impl RasterFilePlan {
    pub fn new(
        asset: RasterMaskAsset,
        operation_instance: u128,
    ) -> Result<Self, RasterFileExecutionError> {
        let roi = MaskRoi::full(asset.width(), asset.height());
        let producer = ProducerIdentity::new(
            operation_instance,
            1,
            asset.identity(),
            asset.original_byte_hash(),
            asset.decoded_mask_hash(),
            roi,
            1.0,
        )
        .map_err(|error| RasterFileExecutionError::MaskModel(error.to_string()))?;
        let descriptor = RasterMaskDescriptor::new(
            MaskIdentity::new(operation_instance, 0, operation_instance, 1),
            producer,
        );
        let receipt = RasterFileReceipt {
            identity: asset.identity(),
            format: asset.format(),
            dimensions: (asset.width(), asset.height()),
            mode: asset.mode(),
            mask_hash: asset.decoded_mask_hash(),
        };
        Ok(Self {
            asset,
            descriptor,
            receipt,
        })
    }

    #[must_use]
    pub const fn asset(&self) -> &RasterMaskAsset {
        &self.asset
    }
    #[must_use]
    pub const fn descriptor(&self) -> &RasterMaskDescriptor {
        &self.descriptor
    }
    #[must_use]
    pub const fn receipt(&self) -> &RasterFileReceipt {
        &self.receipt
    }
    #[must_use]
    pub const fn mask(&self) -> &rusttable_masks::MaskRaster {
        self.asset.mask()
    }

    pub fn validate_input_dimensions(
        &self,
        dimensions: (u32, u32),
    ) -> Result<(), RasterFileExecutionError> {
        if dimensions != self.receipt.dimensions {
            return Err(RasterFileExecutionError::DimensionMismatch {
                input: dimensions,
                asset: self.receipt.dimensions,
            });
        }
        Ok(())
    }

    pub fn publish(&self) -> Result<RasterMaskPublication, RasterFileExecutionError> {
        RasterMaskPublication::new(self.descriptor.clone(), self.asset.mask().clone())
            .map_err(|error| RasterFileExecutionError::MaskModel(error.to_string()))
    }

    pub fn tile(&self, roi: Roi) -> Result<RasterFileTile, RasterFileExecutionError> {
        let x_end = roi
            .x()
            .checked_add(roi.width())
            .ok_or(RasterFileExecutionError::ArithmeticOverflow)?;
        let y_end = roi
            .y()
            .checked_add(roi.height())
            .ok_or(RasterFileExecutionError::ArithmeticOverflow)?;
        if x_end > self.asset.width()
            || y_end > self.asset.height()
            || roi.width() == 0
            || roi.height() == 0
        {
            return Err(RasterFileExecutionError::InvalidRoi);
        }
        let width = usize::try_from(self.asset.width())
            .map_err(|_| RasterFileExecutionError::ArithmeticOverflow)?;
        let tile_width = usize::try_from(roi.width())
            .map_err(|_| RasterFileExecutionError::ArithmeticOverflow)?;
        let mut values = Vec::with_capacity(
            tile_width
                .checked_mul(
                    usize::try_from(roi.height())
                        .map_err(|_| RasterFileExecutionError::ArithmeticOverflow)?,
                )
                .ok_or(RasterFileExecutionError::ArithmeticOverflow)?,
        );
        for y in roi.y()..y_end {
            let start = usize::try_from(y)
                .map_err(|_| RasterFileExecutionError::ArithmeticOverflow)?
                .checked_mul(width)
                .and_then(|offset| offset.checked_add(usize::try_from(roi.x()).ok()?))
                .ok_or(RasterFileExecutionError::ArithmeticOverflow)?;
            let end = start
                .checked_add(tile_width)
                .ok_or(RasterFileExecutionError::ArithmeticOverflow)?;
            values.extend_from_slice(
                self.asset
                    .mask()
                    .values()
                    .get(start..end)
                    .ok_or(RasterFileExecutionError::InvalidRoi)?,
            );
        }
        let mask = rusttable_masks::MaskRaster::new(roi.width(), roi.height(), values)
            .map_err(|error| RasterFileExecutionError::MaskModel(error.to_string()))?;
        Ok(RasterFileTile { roi, mask })
    }

    /// Creates detached vector forms from the frozen mask at the compatibility threshold.
    pub fn vectorize(
        &self,
    ) -> Result<(Vec<RasterFileForm>, RasterFileVectorizationReceipt), RasterFileExecutionError>
    {
        let width = usize::try_from(self.asset.width())
            .map_err(|_| RasterFileExecutionError::ArithmeticOverflow)?;
        let height = usize::try_from(self.asset.height())
            .map_err(|_| RasterFileExecutionError::ArithmeticOverflow)?;
        let values = self.asset.mask().values();
        let mut visited = vec![false; values.len()];
        let mut forms = Vec::new();
        for y in 0..height {
            for x in 0..width {
                let index = y
                    .checked_mul(width)
                    .and_then(|row| row.checked_add(x))
                    .ok_or(RasterFileExecutionError::ArithmeticOverflow)?;
                if visited[index] || values[index] < 0.6 {
                    continue;
                }
                let mut queue = VecDeque::from([(x, y)]);
                visited[index] = true;
                let mut left = x;
                let mut top = y;
                let mut right = x;
                let mut bottom = y;
                while let Some((cx, cy)) = queue.pop_front() {
                    left = left.min(cx);
                    top = top.min(cy);
                    right = right.max(cx);
                    bottom = bottom.max(cy);
                    for (nx, ny) in neighbors(cx, cy, width, height) {
                        let nindex = ny * width + nx;
                        if !visited[nindex] && values[nindex] >= 0.6 {
                            visited[nindex] = true;
                            queue.push_back((nx, ny));
                        }
                    }
                }
                let bounds = (
                    u32::try_from(left)
                        .map_err(|_| RasterFileExecutionError::ArithmeticOverflow)?,
                    u32::try_from(top).map_err(|_| RasterFileExecutionError::ArithmeticOverflow)?,
                    u32::try_from(
                        right
                            .checked_add(1)
                            .ok_or(RasterFileExecutionError::ArithmeticOverflow)?,
                    )
                    .map_err(|_| RasterFileExecutionError::ArithmeticOverflow)?,
                    u32::try_from(
                        bottom
                            .checked_add(1)
                            .ok_or(RasterFileExecutionError::ArithmeticOverflow)?,
                    )
                    .map_err(|_| RasterFileExecutionError::ArithmeticOverflow)?,
                );
                let mut hasher = Sha256::new();
                hasher.update(b"rusttable.rasterfile.form.v1");
                hasher.update(self.asset.identity());
                hasher.update(0.6_f32.to_bits().to_le_bytes());
                for bound in [bounds.0, bounds.1, bounds.2, bounds.3] {
                    hasher.update(bound.to_le_bytes());
                }
                forms.push(RasterFileForm {
                    id: u32::try_from(
                        forms
                            .len()
                            .checked_add(1)
                            .ok_or(RasterFileExecutionError::ArithmeticOverflow)?,
                    )
                    .map_err(|_| RasterFileExecutionError::ArithmeticOverflow)?,
                    bounds,
                    geometry_hash: hasher.finalize().into(),
                });
            }
        }
        let mut hasher = Sha256::new();
        for form in &forms {
            hasher.update(form.geometry_hash);
        }
        let receipt = RasterFileVectorizationReceipt {
            form_count: u32::try_from(forms.len())
                .map_err(|_| RasterFileExecutionError::ArithmeticOverflow)?,
            geometry_hashes: forms.iter().map(|form| form.geometry_hash).collect(),
            transaction_hash: hasher.finalize().into(),
        };
        Ok((forms, receipt))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RasterFileTile {
    roi: Roi,
    mask: rusttable_masks::MaskRaster,
}
impl RasterFileTile {
    #[must_use]
    pub const fn roi(&self) -> Roi {
        self.roi
    }
    #[must_use]
    pub const fn mask(&self) -> &rusttable_masks::MaskRaster {
        &self.mask
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasterFileForm {
    id: u32,
    bounds: (u32, u32, u32, u32),
    geometry_hash: [u8; 32],
}
impl RasterFileForm {
    #[must_use]
    pub const fn id(&self) -> u32 {
        self.id
    }
    #[must_use]
    pub const fn bounds(&self) -> (u32, u32, u32, u32) {
        self.bounds
    }
    #[must_use]
    pub const fn geometry_hash(&self) -> [u8; 32] {
        self.geometry_hash
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasterFileVectorizationReceipt {
    form_count: u32,
    geometry_hashes: Vec<[u8; 32]>,
    transaction_hash: [u8; 32],
}
impl RasterFileVectorizationReceipt {
    #[must_use]
    pub const fn form_count(&self) -> u32 {
        self.form_count
    }
    #[must_use]
    pub fn geometry_hashes(&self) -> &[[u8; 32]] {
        &self.geometry_hashes
    }
    #[must_use]
    pub const fn transaction_hash(&self) -> [u8; 32] {
        self.transaction_hash
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RasterFileExecutionError {
    ArithmeticOverflow,
    InvalidRoi,
    DimensionMismatch {
        input: (u32, u32),
        asset: (u32, u32),
    },
    MaskModel(String),
}
impl fmt::Display for RasterFileExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ArithmeticOverflow => {
                formatter.write_str("rasterfile execution arithmetic overflowed")
            }
            Self::InvalidRoi => {
                formatter.write_str("rasterfile tile ROI is outside the frozen source mask")
            }
            Self::DimensionMismatch { input, asset } => write!(
                formatter,
                "rasterfile input dimensions {input:?} differ from asset {asset:?}"
            ),
            Self::MaskModel(reason) => {
                write!(formatter, "rasterfile mask publication failed: {reason}")
            }
        }
    }
}
impl std::error::Error for RasterFileExecutionError {}

pub fn wgpu_dispatch(width: u32, height: u32) -> Result<u32, RasterFileExecutionError> {
    if width == 0 || height == 0 {
        return Err(RasterFileExecutionError::InvalidRoi);
    }
    let work_items = width
        .checked_mul(height)
        .ok_or(RasterFileExecutionError::ArithmeticOverflow)?;
    work_items
        .checked_add(63)
        .map(|value| value / 64)
        .ok_or(RasterFileExecutionError::ArithmeticOverflow)
}

fn neighbors(x: usize, y: usize, width: usize, height: usize) -> [(usize, usize); 4] {
    [
        (x.checked_sub(1).unwrap_or(x), y),
        (
            x.checked_add(1).filter(|next| *next < width).unwrap_or(x),
            y,
        ),
        (x, y.checked_sub(1).unwrap_or(y)),
        (
            x,
            y.checked_add(1).filter(|next| *next < height).unwrap_or(y),
        ),
    ]
}
