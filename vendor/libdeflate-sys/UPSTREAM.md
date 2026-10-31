# Strict libdeflate source ownership

This directory contains `libdeflate-sys` **1.26.1**, including its bundled native
libdeflate **1.26**. mcmap's maintained fastanvil dependency and bounded PNG
encoder select this path. The root Cargo lockfile controls the application build.

- Rust bindings repository: https://github.com/libdeflater/libdeflater
- Rust bindings commit: `260e1e9a333e32e769855c1ad1f203289e0a3c8d`
- Upstream directory: `libdeflate-sys/`
- Registry archive: https://crates.io/api/v1/crates/libdeflate-sys/1.26.1/download
- Archive SHA-256: `d7870e5fbd2766179a937c725fb11f4ca0ef025d982beb61bd3ce755425bd19c`
- Native repository: https://github.com/ebiggers/libdeflate

The Rust bindings use Apache-2.0, retained in [LICENSE](LICENSE). The native
implementation uses MIT, retained in [libdeflate/COPYING](libdeflate/COPYING)
and its source-file notices. The published crate supplies the import baseline;
`Cargo.toml.orig` and `.cargo_vcs_info.json` preserve upstream metadata.

## Validation contract

Upstream libdeflate intentionally accepts some DEFLATE encodings rejected by
zlib-compatible decoders. A matching Adler-32 or gzip CRC does not detect this:
reserved symbols can produce the same output as a legal encoding. mcmap uses
libdeflate only as a fast path whose accepted inputs preserve the existing
streaming decoder's results. Rejected native inputs return to that decoder,
including historical tolerance of some truncated streams.

`libdeflate/lib/deflate_decompress.c` and `decompress_template.h` implement:

- Reserved literal/length symbols 286 and 287 use invalid entries on the
  existing exceptional path; reserved distance symbols 30 and 31 use distance
  zero, rejected by the existing distance bounds checks.
- Dynamic headers permit at most 286 literal/length and 30 distance symbols,
  and the end-of-block symbol must have a codeword.
- Incomplete precodes are rejected. Legal single-codeword literal/distance
  trees retain invalid entries for the unused codeword. Empty distance trees
  contain invalid entries and remain usable when no distance is decoded.

`build.rs` forbids the `dynamic` feature so an unpatched system library cannot
silently replace these checks. The native runtime CPU dispatch remains intact.
No CMake dependency or additional worker threads are introduced.

The caller in `../fastanvil/src/region/decompression.rs` delegates nonstandard
zlib window declarations and optional gzip headers to the original streaming
implementation. It checks complete input consumption and retains that decoder
as the fallback for every native rejection.

## Refresh and verification

1. Download the upstream crate and verify its registry archive checksum.
2. Compare its pristine sources with this directory and deliberately preserve
   or retire each validation rule. Do not overwrite this copy blindly.
3. Retain both licenses and record the version, commit and archive checksum.
4. Run `cargo test --locked -p fastanvil --lib --features flate2/zlib-rs
   test::region_decompression::` and the equivalent default-backend filter.
5. Compare acceptance, exact output and IO errors against the streaming path,
   including reserved symbols, malformed trees, headers, truncated streams and
   the saved single-bit regression fixture.
6. Compare repeated single-thread release measurements and identical pixels
   on real region files. Confirm Windows/MSVC, GNU and musl builds in CI.

The initial strict implementation passed 42,180 generated gzip/zlib
truncation, bit-flip and gzip-flag comparisons against `flate2/zlib-rs`, plus
targeted synthetic validation cases. On the same 1024-chunk samples, strict
native decoding took approximately 13 ms for the 26.3 region and 21.5 ms for
the GTNH region, versus approximately 20.6/33.3 ms for a reused zlib-rs decoder
and 22.0/36.3 ms for a reused zlib-ng decoder. These are decompression-only
measurements, not complete render timings.

Application behavior and full rendering evidence are owned by
[Minecraft compatibility](../../MINECRAFT_COMPATIBILITY.md).
