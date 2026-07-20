// Shared fixture constructors are intentionally unused by some integration-test binaries.
#![allow(dead_code)]

pub fn tiff_with_metadata() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(220);
    bytes.extend_from_slice(b"II*\0");
    put_u32(&mut bytes, 8);
    put_u16(&mut bytes, 4);
    put_entry(&mut bytes, 0x010f, 2, 6, 62);
    put_entry(&mut bytes, 0x0110, 2, 6, 68);
    put_entry(&mut bytes, 0x0112, 3, 1, 1);
    put_entry(&mut bytes, 0x8769, 4, 1, 74);
    put_u32(&mut bytes, 0);
    bytes.extend_from_slice(b"Canon\0EOS R\0");
    put_u16(&mut bytes, 6);
    put_entry(&mut bytes, 0x9003, 2, 20, 152);
    put_entry(&mut bytes, 0x829a, 5, 1, 172);
    put_entry(&mut bytes, 0x829d, 5, 1, 180);
    put_entry(&mut bytes, 0x920a, 5, 1, 188);
    put_entry(&mut bytes, 0x8833, 3, 1, 400);
    put_entry(&mut bytes, 0xa434, 2, 8, 196);
    put_u32(&mut bytes, 0);
    bytes.extend_from_slice(b"2024:01:02 03:04:05\0");
    put_u32(&mut bytes, 1);
    put_u32(&mut bytes, 125);
    put_u32(&mut bytes, 28);
    put_u32(&mut bytes, 10);
    put_u32(&mut bytes, 50);
    put_u32(&mut bytes, 1);
    bytes.extend_from_slice(b"RF 50mm\0");
    bytes
}

pub fn jpeg_with_exif() -> Vec<u8> {
    let tiff = tiff_with_metadata();
    let mut bytes = vec![0xff, 0xd8, 0xff, 0xe1];
    let segment_len = u16::try_from(tiff.len() + 8).expect("fixture fits JPEG segment");
    bytes.extend_from_slice(&segment_len.to_be_bytes());
    bytes.extend_from_slice(b"Exif\0\0");
    bytes.extend_from_slice(&tiff);
    bytes.extend_from_slice(&[0xff, 0xd9]);
    bytes
}

pub fn png_with_exif() -> Vec<u8> {
    let tiff = tiff_with_metadata();
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    put_chunk(&mut bytes, *b"eXIf", &tiff);
    put_chunk(&mut bytes, *b"IEND", &[]);
    bytes
}

fn put_chunk(bytes: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) {
    bytes.extend_from_slice(
        &u32::try_from(data.len())
            .expect("fixture fits PNG")
            .to_be_bytes(),
    );
    bytes.extend_from_slice(&kind);
    bytes.extend_from_slice(data);
    bytes.extend_from_slice(&[0; 4]);
}

fn put_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
fn put_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
fn put_entry(bytes: &mut Vec<u8>, tag: u16, kind: u16, count: u32, value: u32) {
    put_u16(bytes, tag);
    put_u16(bytes, kind);
    put_u32(bytes, count);
    put_u32(bytes, value);
}
