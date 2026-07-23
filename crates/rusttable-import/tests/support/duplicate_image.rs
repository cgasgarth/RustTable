use std::path::Path;

use rusttable_image::{
    DecodedImage, ImageDimensions, ImageInput, ImageInputError, ImageProbe, InputFormat,
};

pub struct ConstantImageInput;

impl ConstantImageInput {
    fn image() -> DecodedImage {
        DecodedImage::new(ImageDimensions::new(9, 8).unwrap(), vec![127; 9 * 8 * 4]).unwrap()
    }

    fn probe() -> ImageProbe {
        ImageProbe::new(InputFormat::Png, ImageDimensions::new(9, 8).unwrap())
    }
}

impl ImageInput for ConstantImageInput {
    fn probe_bytes(&self, _bytes: &[u8]) -> Result<ImageProbe, ImageInputError> {
        Ok(Self::probe())
    }

    fn decode_bytes(&self, _bytes: &[u8]) -> Result<DecodedImage, ImageInputError> {
        Ok(Self::image())
    }

    fn probe_path(&self, _path: &Path) -> Result<ImageProbe, ImageInputError> {
        Ok(Self::probe())
    }

    fn decode_path(&self, _path: &Path) -> Result<DecodedImage, ImageInputError> {
        Ok(Self::image())
    }
}
