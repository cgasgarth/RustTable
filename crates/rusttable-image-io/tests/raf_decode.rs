use rusttable_image::{ColorEncoding, DecodeLimits, ImageInput, ImageInputError, InputFormat};
use rusttable_image_io::FileImageInput;

fn limits() -> DecodeLimits {
    DecodeLimits::new(1_000_000, 2_000, 2_000, 10_000, 40_000).expect("limits")
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn put_entry(bytes: &mut [u8], offset: usize, tag: u16, ty: u16, count: u32, value: u32) {
    put_u16(bytes, offset, tag);
    put_u16(bytes, offset + 2, ty);
    put_u32(bytes, offset + 4, count);
    put_u32(bytes, offset + 8, value);
}

fn synthetic_compressed_raf() -> Vec<u8> {
    let mut bytes = vec![0; 70_000];
    bytes[..16].copy_from_slice(b"FUJIFILMCCD-RAW ");
    bytes[28..36].copy_from_slice(b"FUJIFILM");
    put_u32(&mut bytes, 84, 200);
    put_u32(&mut bytes, 92, 300);
    put_u32(&mut bytes, 100, 400);

    bytes[212..220].copy_from_slice(&[b'M', b'M', 0, 42, 0, 0, 0, 8]);
    put_u16(&mut bytes, 220, 3);
    put_entry(&mut bytes, 222, 0x010f, 2, 9, 338);
    put_entry(&mut bytes, 234, 0x0110, 2, 7, 348);
    put_entry(&mut bytes, 246, 0x8769, 4, 1, 388);
    put_u32(&mut bytes, 258, 0);
    bytes[550..559].copy_from_slice(b"FUJIFILM\0");
    bytes[560..567].copy_from_slice(b"X-Pro2\0");

    put_u16(&mut bytes, 600, 1);
    put_entry(&mut bytes, 602, 0x927c, 7, 16, 488);
    bytes[700..708].copy_from_slice(b"FUJIFILM");
    put_u32(&mut bytes, 708, 0);
    put_u16(&mut bytes, 712, 0);

    put_u32(&mut bytes, 300, 1);
    put_u16(&mut bytes, 304, 0x0131);
    put_u16(&mut bytes, 306, 36);
    for (index, color) in b"RBGBRGGGRGGBGGBGGRBRGRBGGGBGGRGGRGGB"
        .iter()
        .rev()
        .enumerate()
    {
        bytes[308 + index] = match color {
            b'R' => 0,
            b'G' => 1,
            b'B' => 2,
            _ => unreachable!("synthetic CFA uses RGB only"),
        };
    }

    bytes[400..408].copy_from_slice(&[b'M', b'M', 0, 42, 0, 0, 0, 8]);
    put_u16(&mut bytes, 408, 1);
    put_entry(&mut bytes, 410, 0xf000, 4, 1, 42);
    put_u32(&mut bytes, 422, 0);

    put_u16(&mut bytes, 442, 6);
    put_entry(&mut bytes, 444, 0xf001, 4, 1, 768);
    put_entry(&mut bytes, 456, 0xf002, 4, 1, 6);
    put_entry(&mut bytes, 468, 0xf003, 3, 1, 14 << 16);
    put_entry(&mut bytes, 480, 0xf007, 4, 1, 600);
    put_entry(&mut bytes, 492, 0xf008, 4, 1, 65_568);
    put_entry(&mut bytes, 504, 0xf00e, 3, 4, 320);
    put_u32(&mut bytes, 516, 0);
    put_u16(&mut bytes, 720, 1_024);
    put_u16(&mut bytes, 722, 1_024);
    put_u16(&mut bytes, 724, 1_024);
    put_u16(&mut bytes, 726, 1_024);

    bytes[1_000..1_002].copy_from_slice(b"IS");
    bytes[1_002..1_005].copy_from_slice(&[1, 16, 14]);
    put_u16(&mut bytes, 1_005, 6);
    put_u16(&mut bytes, 1_007, 768);
    put_u16(&mut bytes, 1_009, 768);
    put_u16(&mut bytes, 1_011, 768);
    bytes[1_013] = 1;
    put_u16(&mut bytes, 1_014, 1);
    put_u32(&mut bytes, 1_016, 65_536);
    bytes[1_032..66_568].fill(0xff);
    bytes
}

#[test]
fn compressed_raf_probe_and_decode_are_deterministic() {
    let input = FileImageInput::new(limits());
    let bytes = synthetic_compressed_raf();

    let probe = input.probe_bytes(&bytes).expect("compressed RAF probe");
    let decoded = input.decode_bytes(&bytes).expect("compressed RAF decode");
    let decoded_again = input
        .decode_bytes(&bytes)
        .expect("repeat compressed RAF decode");

    assert_eq!(probe.format(), InputFormat::Raw);
    assert_eq!(probe.dimensions().width(), 768);
    assert_eq!(probe.dimensions().height(), 6);
    assert_eq!(decoded.dimensions().width(), 640);
    assert_eq!(decoded.dimensions().height(), 6);
    assert_eq!(decoded.color_encoding(), ColorEncoding::Srgb);
    assert_eq!(decoded.pixels().len(), 640 * 6 * 4);
    assert_eq!(decoded.pixels(), decoded_again.pixels());
    let mean_rgb = decoded
        .pixels()
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|pixel| &pixel[..3])
        .map(|channel| f64::from(*channel))
        .sum::<f64>()
        / f64::from(decoded.dimensions().width() * decoded.dimensions().height() * 3);
    assert!(
        mean_rgb >= 32.0,
        "developed RAW must not be near-black; mean RGB was {mean_rgb:.2}"
    );
}

#[test]
fn truncated_compressed_raf_returns_typed_error() {
    let input = FileImageInput::new(limits());
    let mut bytes = synthetic_compressed_raf();
    bytes.truncate(1_032);

    let error = input
        .decode_bytes(&bytes)
        .expect_err("truncated compressed RAF must fail");
    assert!(matches!(
        error,
        ImageInputError::MalformedInput {
            format: InputFormat::Raw,
            ..
        }
    ));
}
