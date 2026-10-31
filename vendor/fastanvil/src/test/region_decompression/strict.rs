use super::{compressed, streaming_decode};
use crate::{CompressionScheme, Region};
use std::io::Cursor;

#[derive(Default)]
struct Bits {
    bytes: Vec<u8>,
    used: usize,
}

impl Bits {
    fn put(&mut self, value: u16, count: usize) {
        for bit in 0..count {
            if self.used % 8 == 0 {
                self.bytes.push(0);
            }
            let last = self.bytes.len() - 1;
            self.bytes[last] |= (((value >> bit) & 1) as u8) << (self.used % 8);
            self.used += 1;
        }
    }

    fn code(&mut self, value: u16, count: usize) {
        self.put(value.reverse_bits() >> (16 - count), count);
    }

    fn fixed(&mut self, symbol: u16) {
        match symbol {
            0..=143 => self.code(0x30 + symbol, 8),
            144..=255 => self.code(0x190 + symbol - 144, 9),
            256..=279 => self.code(symbol - 256, 7),
            280..=287 => self.code(0xC0 + symbol - 280, 8),
            _ => unreachable!(),
        }
    }
}

fn dynamic(literals: &[u8], distances: &[u8]) -> Bits {
    let mut bits = Bits::default();
    bits.put(5, 3);
    bits.put((literals.len() - 257) as u16, 5);
    bits.put((distances.len() - 1) as u16, 5);
    bits.put(14, 4);
    for symbol in [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1] {
        bits.put(
            match symbol {
                0 => 1,
                1 | 2 => 2,
                _ => 0,
            },
            3,
        );
    }
    for &length in literals.iter().chain(distances) {
        match length {
            0 => bits.code(0, 1),
            1 => bits.code(2, 2),
            2 => bits.code(3, 2),
            _ => unreachable!(),
        }
    }
    bits
}

fn wrapped(scheme: u8, raw: &[u8], payload: &[u8]) -> Vec<u8> {
    let encoded = compressed(scheme, payload);
    let (header, footer) = if scheme == 1 { (10, 8) } else { (2, 4) };
    let mut bytes = encoded[..header].to_vec();
    bytes.extend(raw);
    bytes.extend(&encoded[encoded.len() - footer..]);
    bytes
}

fn equivalent(scheme: u8, input: &[u8]) {
    let expected = streaming_decode(scheme, input);
    let mut region = Region::create(Cursor::new(Vec::new())).unwrap();
    region
        .write_compressed_chunk(0, 0, CompressionScheme::try_from(scheme).unwrap(), input)
        .unwrap();
    let actual = region.read_chunk(0, 0);
    match expected {
        Ok(expected) => assert_eq!(actual.unwrap().unwrap(), expected),
        Err(expected) => {
            let crate::Error::IO(actual) = actual.unwrap_err() else {
                panic!("expected the original streaming IO error");
            };
            assert_eq!(actual.kind(), expected.kind());
            assert_eq!(actual.to_string(), expected.to_string());
        }
    }
}

fn native_rejects(raw: &[u8]) {
    let mut output = vec![0; 131072];
    let mut consumed = 0;
    let mut written = 0;
    unsafe {
        let decoder = libdeflate_sys::libdeflate_alloc_decompressor();
        assert!(!decoder.is_null());
        let result = libdeflate_sys::libdeflate_deflate_decompress_ex(
            decoder,
            raw.as_ptr().cast(),
            raw.len(),
            output.as_mut_ptr().cast(),
            output.len(),
            &mut consumed,
            &mut written,
        );
        libdeflate_sys::libdeflate_free_decompressor(decoder);
        assert_eq!(
            result,
            libdeflate_sys::libdeflate_result_LIBDEFLATE_BAD_DATA
        );
    }
}

#[test]
fn reserved_fixed_symbols_preserve_streaming_rejection() {
    for symbol in [286, 287] {
        let mut bits = Bits::default();
        bits.put(3, 3);
        bits.fixed(b'a' as u16);
        bits.fixed(symbol);
        bits.code(0, 5);
        bits.fixed(256);
        native_rejects(&bits.bytes);
        for scheme in [1, 2] {
            equivalent(scheme, &wrapped(scheme, &bits.bytes, &vec![b'a'; 259]));
        }
    }
    for symbol in [30, 31] {
        let mut bits = Bits::default();
        bits.put(3, 3);
        bits.fixed(b'a' as u16);
        for _ in 0..100 {
            bits.fixed(285);
            bits.code(0, 5);
        }
        bits.fixed(257);
        bits.code(symbol, 5);
        bits.put(0, 13);
        bits.fixed(256);
        native_rejects(&bits.bytes);
        for scheme in [1, 2] {
            equivalent(scheme, &wrapped(scheme, &bits.bytes, &vec![b'a'; 25804]));
        }
    }
}

#[test]
fn dynamic_reserved_counts_and_unused_codes_preserve_rejection() {
    let mut cases = Vec::new();
    for (nlen, ndist) in [(287, 1), (288, 1), (257, 31), (257, 32)] {
        let mut lengths = vec![0; nlen];
        lengths[65] = 1;
        lengths[256] = 1;
        let mut bits = dynamic(&lengths, &vec![0; ndist]);
        bits.code(0, 1);
        bits.code(1, 1);
        cases.push((bits.bytes, vec![b'A']));
    }
    let mut lengths = vec![0; 257];
    lengths[256] = 1;
    let mut bits = dynamic(&lengths, &[0]);
    bits.code(1, 1);
    cases.push((bits.bytes, vec![]));
    for distance_length in [0, 1] {
        let mut lengths = vec![0; 258];
        lengths[65] = 2;
        lengths[256] = 2;
        lengths[257] = 1;
        let mut bits = dynamic(&lengths, &[distance_length]);
        bits.code(2, 2);
        bits.code(0, 1);
        bits.code(1, 1);
        bits.code(3, 2);
        cases.push((bits.bytes, vec![b'A'; 4]));
    }
    for has_end in [false, true] {
        let mut lengths = vec![0; 257];
        lengths[65] = 1;
        lengths[66] = 1;
        lengths[256] = u8::from(has_end);
        let mut bits = dynamic(&lengths, &[0]);
        bits.code(0, 1);
        cases.push((bits.bytes, vec![b'A']));
    }
    for (raw, payload) in cases {
        native_rejects(&raw);
        for scheme in [1, 2] {
            equivalent(scheme, &wrapped(scheme, &raw, &payload));
        }
    }
}

#[test]
fn saved_single_bit_mutation_keeps_the_streaming_result() {
    let gzip = include_bytes!("../../../resources/strict_reserved_litlen.gzip");
    native_rejects(&gzip[10..gzip.len() - 8]);
    equivalent(1, gzip);
}

#[test]
fn zlib_declared_window_and_headers_preserve_streaming_semantics() {
    let mut bits = Bits::default();
    let payload: Vec<_> = (0..300).map(|i| i as u8).chain([0, 1, 2]).collect();
    bits.put(3, 3);
    for &byte in &payload[..300] {
        bits.fixed(byte as u16);
    }
    bits.fixed(257);
    bits.code(16, 5);
    bits.put(43, 7);
    bits.fixed(256);
    for cinfo in 0u16..=15 {
        for fdict in [0u16, 32] {
            let mut bytes = wrapped(2, &bits.bytes, &payload);
            let cmf = (cinfo << 4) | 8;
            let base = (cmf << 8) | fdict;
            let header = base + (31 - base % 31) % 31;
            bytes[..2].copy_from_slice(&header.to_be_bytes());
            equivalent(2, &bytes);
            bytes[1] ^= 1;
            equivalent(2, &bytes);
        }
    }
}
