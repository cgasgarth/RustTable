use rusttable_image::{
    DecodedImage, ImageDimensions, ImageOutput, JpegQuality, OutputLimits, OutputOptions,
};
use rusttable_image_io::FileImageOutput;
use std::fs;

#[test]
fn generated_output_does_not_copy_user_metadata_markers() {
    let image = DecodedImage::new(ImageDimensions::new(1, 1).unwrap(), vec![1, 2, 3, 255]).unwrap();
    for (name, options) in [
        ("png", OutputOptions::Png),
        (
            "jpeg",
            OutputOptions::Jpeg {
                quality: JpegQuality::new(90).expect("quality"),
            },
        ),
        ("tiff", OutputOptions::Tiff),
    ] {
        let destination = std::env::temp_dir().join(format!("rusttable-output-metadata-{name}"));
        FileImageOutput::new(OutputLimits::new(1_000_000).unwrap())
            .write_new(&image, &destination, options)
            .unwrap();
        let bytes = fs::read(&destination).unwrap();
        for marker in [
            b"Exif\0".as_slice(),
            b"XMP".as_slice(),
            b"iCCP".as_slice(),
            b"IPTC".as_slice(),
        ] {
            assert!(!bytes.windows(marker.len()).any(|window| window == marker));
        }
        fs::remove_file(destination).unwrap();
    }
}
