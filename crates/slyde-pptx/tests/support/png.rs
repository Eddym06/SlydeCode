//! Test-only fixture: a minimal, dependency-free solid-colour PNG encoder.
//!
//! Shared by the crate's unit tests (through `#[path]`) and by the integration
//! test that produces `target/tmp/pptx-check.pptx` for the python-pptx
//! validator. It writes an uncompressed (stored) zlib stream so the fixture
//! needs nothing beyond CRC-32 and Adler-32, which are not worth a dependency.
#![allow(dead_code)]

/// Encodes a `width` x `height` PNG filled with `rgb`.
pub fn solid(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    let mut row = Vec::with_capacity(((width * 3) + 1) as usize);
    row.push(0); // filter type 0 (none)
    for _ in 0..width {
        row.extend_from_slice(&rgb);
    }
    let mut scanlines = Vec::with_capacity(row.len() * height as usize);
    for _ in 0..height {
        scanlines.extend_from_slice(&row);
    }

    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    // 8-bit depth, colour type 2 (truecolour), deflate, adaptive filtering,
    // no interlacing.
    header.extend_from_slice(&[8, 2, 0, 0, 0]);

    let mut png = Vec::new();
    png.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    png.extend_from_slice(&chunk(*b"IHDR", &header));
    png.extend_from_slice(&chunk(*b"IDAT", &zlib_stored(&scanlines)));
    png.extend_from_slice(&chunk(*b"IEND", &[]));
    png
}

fn chunk(kind: [u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 12);
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(&kind);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(data.len() + 4);
    crc_input.extend_from_slice(&kind);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
    out
}

fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    if data.is_empty() {
        out.extend_from_slice(&[1, 0x00, 0x00, 0xFF, 0xFF]);
    }
    let mut offset = 0;
    while offset < data.len() {
        let length = (data.len() - offset).min(65_535);
        let last_block = offset + length == data.len();
        out.push(u8::from(last_block));
        out.extend_from_slice(&(length as u16).to_le_bytes());
        out.extend_from_slice(&(!(length as u16)).to_le_bytes());
        out.extend_from_slice(&data[offset..offset + length]);
        offset += length;
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    let mut low = 1u32;
    let mut high = 0u32;
    for &byte in bytes {
        low = (low + u32::from(byte)) % 65_521;
        high = (high + low) % 65_521;
    }
    (high << 16) | low
}
