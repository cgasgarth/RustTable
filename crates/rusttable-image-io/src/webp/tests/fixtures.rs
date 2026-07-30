use image_webp::{ColorType, WebPEncoder};

pub(super) const LOSSY_RED_3X3: &[u8] = &[
    0x52, 0x49, 0x46, 0x46, 0x3c, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50, 0x56, 0x50, 0x38, 0x20,
    0x30, 0x00, 0x00, 0x00, 0xd0, 0x01, 0x00, 0x9d, 0x01, 0x2a, 0x03, 0x00, 0x03, 0x00, 0x02, 0x00,
    0x34, 0x25, 0xa0, 0x02, 0x74, 0xba, 0x01, 0xf8, 0x00, 0x03, 0xb0, 0x00, 0xfe, 0xf0, 0xc4, 0x0b,
    0xff, 0x20, 0xb9, 0x61, 0x75, 0xc8, 0xd7, 0xff, 0x20, 0x3f, 0xe4, 0x07, 0xfc, 0x80, 0xff, 0xf8,
    0xf2, 0x00, 0x00, 0x00,
];

/// Independent 4x3 RGBA lossless fixture produced once by local `cwebp`.
pub(super) const CWEBP_RGBA_4X3: &[u8] = &[
    0x52, 0x49, 0x46, 0x46, 0x3c, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50, 0x56, 0x50, 0x38, 0x4c,
    0x2f, 0x00, 0x00, 0x00, 0x2f, 0x03, 0x80, 0x00, 0x10, 0x5f, 0x20, 0x90, 0x4d, 0xf6, 0xfc, 0xb5,
    0x73, 0x10, 0x10, 0x94, 0x48, 0x56, 0x40, 0x21, 0x80, 0x00, 0x28, 0x68, 0x7a, 0xa0, 0x43, 0xd8,
    0x5b, 0x10, 0x04, 0x01, 0x24, 0x4a, 0x29, 0xa4, 0x30, 0xdc, 0x0a, 0x2b, 0x8c, 0x1a, 0xd1, 0xff,
    0xa0, 0x2e, 0x07, 0x00,
];

/// RGBA bytes emitted by official `dwebp` 1.6.0 for [`CWEBP_RGBA_4X3`].
pub(super) const CWEBP_RGBA_4X3_DWEBP_PIXELS: &[u8] = &[
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
    13, 14, 15, 16, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17,
];

pub(super) fn lossless_rgb() -> (Vec<u8>, Vec<u8>) {
    let pixels = vec![1, 2, 3, 20, 30, 40, 250, 240, 230, 7, 80, 190, 90, 40, 10];
    (encode(&pixels, 5, 1, ColorType::Rgb8, None), pixels)
}

pub(super) fn lossless_rgba() -> (Vec<u8>, Vec<u8>) {
    let pixels = vec![10, 20, 30, 128, 200, 100, 50, 0, 1, 2, 3, 255];
    (encode(&pixels, 3, 1, ColorType::Rgba8, None), pixels)
}

pub(super) fn extended_metadata() -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
    let pixels = vec![9, 8, 7, 6, 5, 4];
    let icc = b"fixture-icc-payload".to_vec();
    let exif = b"Exif\0\0fixture".to_vec();
    let xmp = b"<x:xmpmeta>fixture</x:xmpmeta>".to_vec();
    let source = encode(&pixels, 2, 1, ColorType::Rgb8, Some((&icc, &exif, &xmp)));
    (source, icc, exif, xmp)
}

pub(super) fn extended_with_icc(icc: &[u8]) -> Vec<u8> {
    let pixels = vec![9, 8, 7, 6, 5, 4];
    encode(
        &pixels,
        2,
        1,
        ColorType::Rgb8,
        Some((icc, b"Exif\0\0fixture", b"<x:xmpmeta>fixture</x:xmpmeta>")),
    )
}

pub(super) fn metadata_permutation(order: &[[u8; 4]]) -> Vec<u8> {
    let (encoded, _) = lossless_rgb();
    let image = simple_image_payload(&encoded);
    let mut vp8x = vec![0b0000_1100, 0, 0, 0];
    vp8x.extend_from_slice(&[4, 0, 0, 0, 0, 0]);
    let mut chunks = vec![(*b"VP8X", vp8x)];
    for kind in order {
        chunks.push(match kind {
            b"VP8L" => (*kind, image.clone()),
            b"EXIF" => (*kind, b"Exif\0\0permutation".to_vec()),
            b"XMP " => (*kind, b"<x:xmpmeta>permutation</x:xmpmeta>".to_vec()),
            b"JUNK" => (*kind, b"unknown".to_vec()),
            _ => panic!("unsupported fixture chunk"),
        });
    }
    riff(&chunks)
}

pub(super) fn icc_after_image() -> Vec<u8> {
    let (encoded, _) = lossless_rgb();
    let image = simple_image_payload(&encoded);
    let mut vp8x = vec![0b0010_0000, 0, 0, 0];
    vp8x.extend_from_slice(&[4, 0, 0, 0, 0, 0]);
    riff(&[
        (*b"VP8X", vp8x),
        (*b"VP8L", image),
        (*b"ICCP", b"late-profile".to_vec()),
    ])
}

pub(super) fn metadata_between_alpha_and_image() -> Vec<u8> {
    let vp8 = simple_image_payload(LOSSY_RED_3X3);
    let mut vp8x = vec![0b0001_1000, 0, 0, 0];
    vp8x.extend_from_slice(&[2, 0, 0, 2, 0, 0]);
    let mut alpha = vec![0];
    alpha.extend_from_slice(&[255; 9]);
    riff(&[
        (*b"VP8X", vp8x),
        (*b"ALPH", alpha),
        (*b"EXIF", b"Exif\0\0interposed".to_vec()),
        (*b"VP8 ", vp8),
    ])
}

pub(super) fn lossy_alpha(alpha: &[u8; 9]) -> Vec<u8> {
    let vp8 = simple_image_payload(LOSSY_RED_3X3);
    let mut vp8x = vec![0b0001_0000, 0, 0, 0];
    vp8x.extend_from_slice(&[2, 0, 0, 2, 0, 0]);
    let mut alpha_payload = vec![0];
    alpha_payload.extend_from_slice(alpha);
    riff(&[(*b"VP8X", vp8x), (*b"ALPH", alpha_payload), (*b"VP8 ", vp8)])
}

pub(super) fn animation() -> Vec<u8> {
    let mut vp8x = vec![0b0000_0010, 0, 0, 0];
    vp8x.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    riff(&[(*b"VP8X", vp8x), (*b"ANIM", vec![0, 0, 0, 0, 0, 0])])
}

pub(super) fn odd_unknown_padding() -> Vec<u8> {
    let (_, pixels) = lossless_rgb();
    let encoded = encode(&pixels, 5, 1, ColorType::Rgb8, None);
    let image = simple_image_payload(&encoded);
    let mut vp8x = vec![0, 0, 0, 0];
    vp8x.extend_from_slice(&[4, 0, 0, 0, 0, 0]);
    riff(&[
        (*b"VP8X", vp8x),
        (*b"zzzz", vec![1, 2, 3]),
        (*b"VP8L", image),
    ])
}

pub(super) fn riff(chunks: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut payload = Vec::from(&b"WEBP"[..]);
    for (kind, data) in chunks {
        payload.extend_from_slice(kind);
        payload.extend_from_slice(
            &u32::try_from(data.len())
                .expect("fixture chunk is small")
                .to_le_bytes(),
        );
        payload.extend_from_slice(data);
        if data.len() & 1 != 0 {
            payload.push(0);
        }
    }
    let mut source = Vec::from(&b"RIFF"[..]);
    source.extend_from_slice(
        &u32::try_from(payload.len())
            .expect("fixture RIFF is small")
            .to_le_bytes(),
    );
    source.extend_from_slice(&payload);
    source
}

pub(super) fn simple_image_payload(source: &[u8]) -> Vec<u8> {
    let length = u32::from_le_bytes(source[16..20].try_into().expect("chunk length"));
    let length = usize::try_from(length).expect("fixture length");
    source[20..20 + length].to_vec()
}

fn encode(
    pixels: &[u8],
    width: u32,
    height: u32,
    color: ColorType,
    metadata: Option<(&[u8], &[u8], &[u8])>,
) -> Vec<u8> {
    let mut output = Vec::new();
    let mut encoder = WebPEncoder::new(&mut output);
    if let Some((icc, exif, xmp)) = metadata {
        encoder.set_icc_profile(icc.to_vec());
        encoder.set_exif_metadata(exif.to_vec());
        encoder.set_xmp_metadata(xmp.to_vec());
    }
    encoder
        .encode(pixels, width, height, color)
        .expect("fixture encoding succeeds");
    output
}
