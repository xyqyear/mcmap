use std::io::{self, Cursor, Write};

use flate2::{Compression, Crc, GzBuilder};

use crate::{CompressionScheme, Region};

fn compressed(scheme: u8, input: &[u8], level: Compression) -> Vec<u8> {
    if scheme == 2 {
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), level);
        encoder.write_all(input).unwrap();
        encoder.finish().unwrap()
    } else {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), level);
        encoder.write_all(input).unwrap();
        encoder.finish().unwrap()
    }
}

fn read_payload(scheme: u8, input: &[u8]) -> crate::Result<Vec<u8>> {
    let mut region = Region::create(Cursor::new(Vec::new()))?;
    region.write_compressed_chunk(0, 0, CompressionScheme::try_from(scheme).unwrap(), input)?;
    Ok(region.read_chunk(0, 0)?.unwrap())
}

#[test]
fn valid_zlib_and_gzip_preserve_payloads_at_every_compression_level() {
    for scheme in [1, 2] {
        for length in [0, 1, 257, 65_536, 70_000] {
            let payload: Vec<_> = (0..length).map(|i| (i * 117 + (i >> 2)) as u8).collect();
            for level in 0..=9 {
                let input = compressed(scheme, &payload, Compression::new(level));
                assert_eq!(read_payload(scheme, &input).unwrap(), payload);
            }
        }
    }
}

#[test]
fn first_complete_stream_ends_before_trailing_bytes_or_another_stream() {
    for scheme in [1, 2] {
        let payload = b"first chunk payload";
        let encoded = compressed(scheme, payload, Compression::fast());
        for suffix in [
            vec![0, 1, 2, 3],
            compressed(scheme, b"second chunk payload", Compression::fast()),
        ] {
            let mut input = encoded.clone();
            input.extend(suffix);
            assert_eq!(read_payload(scheme, &input).unwrap(), payload);
        }
    }
}

#[test]
fn invalid_payload_checksums_and_truncation_never_expose_partial_bytes() {
    for scheme in [1, 2] {
        let valid = compressed(scheme, b"complete chunk payload", Compression::fast());
        let mut bad_checksum = valid.clone();
        let index = bad_checksum.len() - if scheme == 1 { 8 } else { 1 };
        bad_checksum[index] ^= 1;
        for input in [
            bad_checksum,
            valid[..valid.len() - 1].to_vec(),
            valid[..valid.len() / 2].to_vec(),
            Vec::new(),
        ] {
            let mut region = Region::create(Cursor::new(Vec::new())).unwrap();
            region
                .write_compressed_chunk(0, 0, CompressionScheme::try_from(scheme).unwrap(), &input)
                .unwrap();
            region
                .write_compressed_chunk(1, 0, CompressionScheme::try_from(scheme).unwrap(), &valid)
                .unwrap();
            let mut output = vec![99; 16];
            let crate::Error::IO(error) = region.read_chunk_into(0, 0, &mut output).unwrap_err()
            else {
                panic!("expected an IO decoding error");
            };
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(output.is_empty());
            assert!(region.read_chunk_into(1, 0, &mut output).unwrap());
            assert_eq!(output, b"complete chunk payload");
        }
    }
}

#[test]
fn valid_gzip_optional_headers_and_long_names_preserve_payloads() {
    let payload = b"gzip metadata does not change the chunk payload";
    let extra = b"extra metadata";
    let comment = b"chunk comment";
    for name_length in [5, 65_536] {
        let mut encoder = GzBuilder::new()
            .extra(extra.to_vec())
            .filename(vec![b'x'; name_length])
            .comment(comment.to_vec())
            .write(Vec::new(), Compression::fast());
        encoder.write_all(payload).unwrap();
        let mut input = encoder.finish().unwrap();
        assert_eq!(read_payload(1, &input).unwrap(), payload);

        input[3] |= 0x02;
        let header_length = 10 + 2 + extra.len() + name_length + 1 + comment.len() + 1;
        let mut checksum = Crc::new();
        checksum.update(&input[..header_length]);
        input.splice(
            header_length..header_length,
            (checksum.sum() as u16).to_le_bytes(),
        );
        assert_eq!(read_payload(1, &input).unwrap(), payload);
    }
}

#[test]
fn valid_zlib_window_declarations_preserve_payloads() {
    let payload = vec![42; 1024];
    let original = compressed(2, &payload, Compression::fast());
    for window in 0u16..=7 {
        let mut input = original.clone();
        let header = (((window << 4) | 8) << 8) | u16::from(input[1] & 0xe0);
        let header = header + (31 - header % 31) % 31;
        input[..2].copy_from_slice(&header.to_be_bytes());
        assert_eq!(read_payload(2, &input).unwrap(), payload);
    }
}

#[test]
fn unused_distance_lengths_allow_the_full_header_range() {
    // RFC 1951 permits up to 32 distance lengths when reserved symbols are unused.
    for (count, header, repeat) in [(30, 0xdd, 0x13), (31, 0xde, 0x14), (32, 0xdf, 0x15)] {
        let input = [
            0x78, 0x9c, 0x05, header, 0x01, 0x05, 0x00, 0x00, 0x00, 0x00, 0xa0, 0x6d, 0xfd, 0x3f,
            0xd5, repeat, 0x01, 0x00, 0x42, 0x00, 0x42,
        ];
        assert_eq!(
            read_payload(2, &input).unwrap(),
            b"A",
            "distance count {count}"
        );
        let gzip = compressed(1, b"A", Compression::fast());
        let mut wrapped = gzip[..10].to_vec();
        wrapped.extend_from_slice(&input[2..input.len() - 4]);
        wrapped.extend_from_slice(&gzip[gzip.len() - 8..]);
        assert_eq!(
            read_payload(1, &wrapped).unwrap(),
            b"A",
            "distance count {count}"
        );
    }
}

#[test]
fn decompression_reuses_buffers_across_large_small_missing_and_raw_chunks() {
    let mut region = Region::create(Cursor::new(Vec::new())).unwrap();
    let large = vec![42; 2 * 1024 * 1024];
    region.write_chunk(0, 0, &large).unwrap();
    region
        .write_compressed_chunk(
            1,
            0,
            CompressionScheme::Gzip,
            &compressed(1, b"small", Compression::fast()),
        )
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
fn lz4_payloads_still_decode_into_the_reused_output() {
    let payload = vec![17; 200_000];
    let mut compressed = Vec::new();
    lz4_java_wrc::Lz4BlockOutput::new(&mut compressed)
        .write_all(&payload)
        .unwrap();
    let mut region = Region::create(Cursor::new(Vec::new())).unwrap();
    region
        .write_compressed_chunk(0, 0, CompressionScheme::Lz4, &compressed)
        .unwrap();
    let mut output = Vec::new();
    assert!(region.read_chunk_into(0, 0, &mut output).unwrap());
    assert_eq!(output, payload);
}
