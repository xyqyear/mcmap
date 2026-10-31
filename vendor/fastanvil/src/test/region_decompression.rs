use std::io::{Cursor, Write};

use flate2::Compression;

use crate::{CompressionScheme, Region};

mod strict;

fn streaming_decode(scheme: u8, compressed: &[u8]) -> std::io::Result<Vec<u8>> {
    if scheme == 2 {
        let mut decoder = flate2::write::ZlibDecoder::new(Vec::new());
        decoder.write_all(compressed)?;
        decoder.finish()
    } else {
        let mut decoder = flate2::write::GzDecoder::new(Vec::new());
        decoder.write_all(compressed)?;
        decoder.finish()
    }
}

fn compressed(scheme: u8, input: &[u8]) -> Vec<u8> {
    if scheme == 2 {
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(input).unwrap();
        encoder.finish().unwrap()
    } else {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(input).unwrap();
        encoder.finish().unwrap()
    }
}

#[test]
fn decompression_matches_streaming_acceptance_and_bytes() {
    for scheme in [1, 2] {
        let valid = compressed(scheme, b"a complete chunk payload");
        let mut bad_checksum = valid.clone();
        let index = bad_checksum.len() - if scheme == 1 { 8 } else { 1 };
        bad_checksum[index] ^= 1;
        let mut trailing = valid.clone();
        trailing.extend([0, 1, 2, 3]);
        let mut concatenated = valid.clone();
        concatenated.extend_from_slice(&valid);
        for input in [
            valid.clone(),
            bad_checksum,
            trailing,
            concatenated,
            valid[..valid.len() - 1].to_vec(),
            valid[..valid.len() / 2].to_vec(),
            Vec::new(),
        ] {
            let expected = streaming_decode(scheme, &input);
            let mut region = Region::create(Cursor::new(Vec::new())).unwrap();
            region
                .write_compressed_chunk(0, 0, CompressionScheme::try_from(scheme).unwrap(), &input)
                .unwrap();
            let actual = region.read_chunk(0, 0);
            match expected {
                Ok(bytes) => assert_eq!(actual.unwrap().unwrap(), bytes),
                Err(_) => assert!(actual.is_err(), "scheme {scheme} accepted invalid bytes"),
            }
        }
    }
}

#[test]
fn decompression_reuses_buffers_across_large_small_missing_and_raw_chunks() {
    let mut region = Region::create(Cursor::new(Vec::new())).unwrap();
    let large = vec![42; 2 * 1024 * 1024];
    region.write_chunk(0, 0, &large).unwrap();
    region
        .write_compressed_chunk(1, 0, CompressionScheme::Gzip, &compressed(1, b"small"))
        .unwrap();
    region
        .write_compressed_chunk(2, 0, CompressionScheme::Uncompressed, b"raw")
        .unwrap();
    let mut output = Vec::new();
    assert!(region.read_chunk_into(0, 0, &mut output).unwrap());
    assert_eq!(output, large);
    let capacity = output.capacity();
    assert!(region.read_chunk_into(1, 0, &mut output).unwrap());
    assert_eq!(output, b"small");
    assert!(!region.read_chunk_into(3, 0, &mut output).unwrap());
    assert!(output.is_empty());
    assert!(region.read_chunk_into(2, 0, &mut output).unwrap());
    assert_eq!(output, b"raw");
    assert_eq!(output.capacity(), capacity);
    let mut cloned = region.clone();
    assert_eq!(cloned.read_chunk(0, 0).unwrap().unwrap(), large);
}

#[test]
fn decompression_preserves_gzip_header_checksums_and_lz4() {
    let payload = b"gzip optional-header validation";
    let mut gzip = compressed(1, payload);
    gzip[3] |= 0x02;
    gzip.splice(10..10, [0, 0]);
    assert!(streaming_decode(1, &gzip).is_err());
    let mut region = Region::create(Cursor::new(Vec::new())).unwrap();
    region
        .write_compressed_chunk(0, 0, CompressionScheme::Gzip, &gzip)
        .unwrap();
    assert!(region.read_chunk(0, 0).is_err());

    let mut long_name = compressed(1, payload);
    long_name[3] |= 0x08;
    let mut filename = vec![b'x'; 65_536];
    filename.push(0);
    long_name.splice(10..10, filename);
    assert!(streaming_decode(1, &long_name).is_err());
    region
        .write_compressed_chunk(0, 0, CompressionScheme::Gzip, &long_name)
        .unwrap();
    assert!(region.read_chunk(0, 0).is_err());

    let payload = vec![17; 200_000];
    let mut compressed = Vec::new();
    lz4_java_wrc::Lz4BlockOutput::new(&mut compressed)
        .write_all(&payload)
        .unwrap();
    region
        .write_compressed_chunk(1, 0, CompressionScheme::Lz4, &compressed)
        .unwrap();
    let mut output = Vec::new();
    assert!(region.read_chunk_into(1, 0, &mut output).unwrap());
    assert_eq!(output, payload);
}
