use super::plan::RgbAiTile;

#[derive(Debug, Clone, PartialEq)]
pub struct AuxiliaryTensorSpec {
    name: String,
    channels: u32,
    value: f32,
}

impl AuxiliaryTensorSpec {
    pub fn constant(name: &'static str, channels: u32, value: f32) -> Result<Self, TensorError> {
        if name.is_empty() || channels == 0 || !value.is_finite() {
            return Err(TensorError::InvalidAuxiliary);
        }
        Ok(Self {
            name: name.to_owned(),
            channels,
            value,
        })
    }
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub const fn channels(&self) -> u32 {
        self.channels
    }
    #[must_use]
    pub const fn value(&self) -> f32 {
        self.value
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RgbAuxiliaryTensor {
    name: String,
    channels: u32,
    width: u32,
    height: u32,
    data: Vec<f32>,
}

impl RgbAuxiliaryTensor {
    pub fn constant(
        spec: &AuxiliaryTensorSpec,
        width: u32,
        height: u32,
    ) -> Result<Self, TensorError> {
        let count = u64::from(spec.channels)
            .checked_mul(u64::from(width))
            .and_then(|value| value.checked_mul(u64::from(height)))
            .and_then(|value| usize::try_from(value).ok())
            .ok_or(TensorError::ArithmeticOverflow)?;
        Ok(Self {
            name: spec.name.clone(),
            channels: spec.channels,
            width,
            height,
            data: vec![spec.value; count],
        })
    }
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub const fn channels(&self) -> u32 {
        self.channels
    }
    #[must_use]
    pub const fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    #[must_use]
    pub fn data(&self) -> &[f32] {
        &self.data
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RgbAiTileInput {
    tile: RgbAiTile,
    width: u32,
    height: u32,
    nchw_rgb: Vec<f32>,
    auxiliary: Vec<RgbAuxiliaryTensor>,
}

impl RgbAiTileInput {
    pub fn new(
        tile: RgbAiTile,
        width: u32,
        height: u32,
        nchw_rgb: Vec<f32>,
        auxiliary: Vec<Result<RgbAuxiliaryTensor, TensorError>>,
    ) -> Result<Self, TensorError> {
        let plane = u64::from(width)
            .checked_mul(u64::from(height))
            .ok_or(TensorError::ArithmeticOverflow)?;
        let expected = usize::try_from(
            plane
                .checked_mul(3)
                .ok_or(TensorError::ArithmeticOverflow)?,
        )
        .map_err(|_| TensorError::ArithmeticOverflow)?;
        if nchw_rgb.len() != expected || nchw_rgb.iter().any(|value| !value.is_finite()) {
            return Err(TensorError::InvalidTensor);
        }
        let auxiliary = auxiliary.into_iter().collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            tile,
            width,
            height,
            nchw_rgb,
            auxiliary,
        })
    }
    #[must_use]
    pub const fn tile(&self) -> RgbAiTile {
        self.tile
    }
    #[must_use]
    pub const fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    #[must_use]
    pub fn nchw_rgb(&self) -> &[f32] {
        &self.nchw_rgb
    }
    #[must_use]
    pub fn auxiliary(&self) -> &[RgbAuxiliaryTensor] {
        &self.auxiliary
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TensorError {
    InvalidAuxiliary,
    InvalidTensor,
    ArithmeticOverflow,
}

impl std::fmt::Display for TensorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "RGB AI tensor contract error: {self:?}")
    }
}
impl std::error::Error for TensorError {}
