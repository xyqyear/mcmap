# Fastanvil source ownership

This directory is mcmap's maintained copy of the latest non-yanked stable
fastanvil release checked on 2026-10-31: **0.32.0**. Cargo uses this directory
directly; changes here are part of mcmap's build and release.

- Repository: https://github.com/owengage/fastnbt
- Upstream commit: `061147caddc500814db987ec66160b0f07eb3e3f`
- Directory in upstream: `fastanvil/`
- Archive: https://crates.io/api/v1/crates/fastanvil/0.32.0/download
- Archive SHA-256: `68f773cb151aa058dd65e3a119cc0733252abf2942d4d3eed8eee72bd231d5b4`
- The crate declares MIT OR Apache-2.0. The upstream repository supplies the MIT
  license in `LICENSE`, copied here from the pinned commit with its attribution.

The published crate is the import baseline, including its source, tests,
resources, examples and normalized Cargo manifest. `Cargo.toml.orig` and
`.cargo_vcs_info.json` retain the release's upstream metadata. The lockfile at
the mcmap repository root controls application builds.

## When to change this library

Change fastanvil here when the behavior belongs to Minecraft region/chunk
decoding, block-state interpretation, packed indices or generic rendering, and
the direct implementation is simpler or avoids costly workarounds in mcmap.
Minecraft format compatibility and decoding performance are explicitly within
this directory's maintenance scope. An upstream release is not required before
making a justified local fix.

Keep CLI arguments, JSON events, application palette policy, file ownership,
cache invalidation and world-management orchestration in mcmap. Generic NBT
reading, arrays and serialization belong to the workspace's
[mcmap-nbt crate](../../crates/mcmap-nbt/README.md).

## Upstream refresh

1. Check the latest stable release, download its archive and verify the registry
   checksum. Record the release, source commit and archive digest here.
2. Compare a pristine extraction with this maintained tree; retain or retire
   each local behavior deliberately. Never overwrite the directory blindly.
3. Keep the upstream import and behavioral changes distinguishable in Git.
   Preserve licensing, resource files and upstream tests.
4. Run affected library tests and mcmap's CLI compatibility tests locally, then
   the complete integration and prune-inhabited workflows on the pushed SHA.
5. For decoding changes, compare release binaries on identical map content,
   verify identical rendered pixels and unchanged source hashes, and measure
   repeated interleaved wall time, CPU time and peak memory. Record results in
   the owning compatibility documentation.

Local changes must remain focused and documented below. Do not add application
configuration, speculative abstractions or unrelated cleanup to the library.

## Local behavior

`src/java/block.rs` directly decodes legacy compounds, modern compounds and
compact strings. It borrows input strings during construction, merges modern
defaults from the generated static table, and creates each final block without
building a whole-chunk NBT tree. Legacy property semantics and renderer
classification are retained. `src/java/mod.rs` preserves errors from every
attempted chunk layout when all decoders fail.

`src/java/decode/` directly reads current and pre-1.18 chunk fields using
mcmap-nbt. Root routing recognizes `Level`, `Status` and `sections`, skipping
unrelated fields. A successful pre-1.18 decode returns directly only when the
root `Status` field is absent. Failed candidates fall back to modern, pre-1.18
and pre-1.13 Serde schemas in that order, using the same mcmap-nbt parser.
Errors retain the context of every failed layout; parser error wording can
differ from upstream. Both paths share block semantics and the constructors
for section towers and packed data.

`ChunkDecoder` owns a bounded cache of validated block palette entries. Exact
raw keys are compared after hashing, with separate maps for string and compound
tags. The cache holds at most 4,096 entries and 8 MiB of key bytes; individual
keys above 64 KiB bypass it. Parsed block strings are additional memory. Returned
chunks own their data and survive decoder destruction or input-buffer reuse.
mcmap's region pipeline creates and drops the decoder within each render call,
including failures and missing regions. There is no global or thread-local cache.
`JavaChunk::from_bytes` remains available for independent chunk reads.

Known modern default states borrow compiled names and encoded descriptions.
The generated finite index uses `ahash`; arbitrary input states are never cached
globally. Packed block indices retain their original validation and use an exact
bounded reciprocal for section-local division.

Pre-1.18 packed states read recognized padded layouts by index without expanding
all 4,096 entries. Other layouts retain the original continuous-bit expansion
and its first-palette-length cache. The optional expanded array lives on the
heap so untouched sections do not carry an inline 8 KiB cache.

Region reads reuse compressed and decompressed buffers and a `libdeflate`
decoder from the registry `libdeflate-sys` dependency. The root lockfile pins
the upstream source and checksum. Zlib/gzip streams, including optional gzip
headers, use that decoder directly; a rejected stream fails the read without
exposing partial bytes. Decoding stops at the first complete compressed stream.
Invalid compressed inputs follow upstream behavior rather than matching another
decoder's acceptance or error text. `flate2` remains in use for writing chunks.
The allocating `read_chunk` API remains available alongside `read_chunk_into`.

Chunks expose bulk surface heights and a compatible default water-depth method.
Modern water columns locate each section once, preserving the renderer's depth
offset and lower bound. First-layer color blends use finite lookup tables of the
existing floating-point calculation, including its truncation. Opaque background
composition removes only exact multiplication/division by one. Palette lookup
and biome tint selection share one implementation with application adapters.
The renderer can return computed southern heights and defer northern height
queries until the first current pixel has been evaluated.

`src/java/default_states_26_3.rs` is generated by mcmap's
`tools/update_default_states.py`; its source and regeneration instructions are
owned by the compatibility document. Library unit tests and performance checks
use the root Cargo workspace and lockfile. Application releases select only the
mcmap package by default.

See [Minecraft compatibility](../../MINECRAFT_COMPATIBILITY.md) for supported
formats, default-state provenance and the decoding/performance contract.
