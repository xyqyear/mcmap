# Rendering performance

Performance measurements use one region per process and `-j 1`. They include
startup, palette loading, region and north-neighbor reads, decoding, shading and
PNG output. File caches are warm; each invocation starts a new process. Existing
multi-region parallel rendering remains available, but is not used to establish
single-region speedups.

## Preserved behavior

For valid saves, optimizations preserve decoded RGBA pixels, dimensions, palette
selection, source files, CLI options, JSON outcomes and `--preserve-mtime`.
PNG compression may produce different file bytes and sizes. Compare decoded pixels, rather than
PNG file hashes. Both the modern and numeric legacy palette formats remain
supported without conversion or extra options.

Invalid compressed streams follow the upstream decoder's acceptance and error
behavior. A successful decode can accept an encoding that another decoder
rejects; matching every historical rejection is not part of the rendering
contract. Decode failures remain errors, and NBT/layout validation still applies.

The renderer retains historical details that affect pixels: floating-point
truncation during composition, water-depth offsets, missing-chunk north-cache
behavior, legacy nonzero-ID shading heights and the heightmap corrections in
[Minecraft compatibility](MINECRAFT_COMPATIBILITY.md).

## Implementation

- Chunk reads reuse buffers and decompressor state. The upstream
  `libdeflate-sys` registry dependency supplies zlib/gzip decompression and PNG
  compression. Region decoding uses it directly, without a second decoder or
  local patches to the native library.
- Modern default block states borrow compiled templates. Palette colors use a
  per-palette hash map, and modern JSON palettes normally need one parse.
- Packed indices precompute their layout and use an exact bounded reciprocal.
  Recognized pre-1.18 layouts read packed values on demand. A root beginning
  with `Level` can avoid a failed modern parse when its decoded fields prove
  that only the older layout can succeed.
  Legacy ID and metadata unpacking processes both nibbles of each byte together.
- Heightmaps are queried in bulk; water columns locate each section once.
  Successfully rendered chunks retain only their southern boundary heights.
  North-region seed chunks and unfinished chunks defer height evaluation until
  a visible neighbor needs it.
- Exact color tables preserve the original floating-point result, including
  its rounding.
- PNG images with a nonempty, exact RGBA8 buffer of at most 16 MiB use
  libdeflate level 1 with no scanline filter. The existing PNG library writes
  the container, checksums and image header. Larger or unusual buffers retain
  the original Fast/Sub encoder and its memory behavior; native allocation or
  capacity failures also fall back to that encoder. Other output formats keep
  their existing encoders.
- `mimalloc` supplies Rust allocations without worker threads or global C
  allocation overrides. This trades additional process memory for speed.
- One-region and explicit `-j 1` requests avoid starting a Rayon worker pool.

Portable releases do not require AES, AVX2 or a particular host CPU. Native
libraries may choose supported instructions at runtime. The hash implementation
uses its portable fallback where the compilation target lacks accelerated
instructions.

GNU release binaries preserve the glibc 2.34 baseline. Their build and release
workflows use Ubuntu 22.04 and reject ELF files requiring newer glibc symbols
before uploading artifacts. This also applies to bundled native dependencies.
Publication waits for all three platform packages and their compatibility checks.

## Decoder comparison

The 2026-11-01 single-thread comparison used 24 real regions containing 21,483
zlib chunks, including a 45.26 MB decompressed chunk. All compared backends
matched independently decoded bytes with small, exact and oversized output
buffers. Each case and backend retained 27 interleaved measurements on the same
i5-13600KF host. The totals below sum each region's median and include decoder
initialization and growth from a 64 KiB buffer; they are not CLI render times.

| Backend | Total decode time | Relative to upstream libdeflate |
| --- | ---: | ---: |
| Upstream libdeflate 1.26 | 657.57 ms | 1.00x |
| Locally stricter libdeflate 1.26 | 670.01 ms | 1.02x |
| ISA-L 2.32.1, O3 | 869.93 ms | 1.32x |
| zlib-rs 0.5.5 | 956.04 ms | 1.45x |
| zlib-rs 0.6.8 | 959.90 ms | 1.46x |
| zlib-ng 2.3.3 | 1038.92 ms | 1.58x |

Zig 0.17's independent decoder also matched the real chunk bytes when wrapped
with checksum verification. Its fast build took 2.36-2.91 times the upstream
libdeflate time on three representative regions, including checksums. It also
rejected valid headers containing 31 or 32 unused distance-code lengths, so it
cannot replace the decoder for every supported input. These measurements select
the upstream C implementation without introducing another toolchain or local
native-library patches. Gzip was covered separately by synthetic correctness
checks; the real-world timing corpus contained zlib frames only.

## Real-world corpus

Keep private archives, extracted maps, palettes and rendered images outside Git.
Select spatially separated regions across saves, dimensions and densities. A
selected benchmark region must render successfully with the frozen baseline and
contain nonzero-alpha pixels. Record unsupported and transparent inputs as
separate behavior fixtures; do not count them as fast successful regions.

Each complete JSON manifest under `manifests/` identifies its archive and lists
`regions` with relative paths, byte sizes, SHA-256, stored-chunk counts,
dimensions and palette paths. `context_regions` records additional northern
neighbors. Palette assets do not contribute to region byte totals. Preserve
relative paths and neighboring files when copying the corpus to the benchmark
filesystem. Do not compare network/NTFS inputs with native-filesystem inputs.

Freeze a release binary before editing code. Generate its reference outputs:

```sh
uv run tools/benchmark_corpus.py \
  --corpus /data/mcmap-corpus --binary /data/bin/baseline \
  --output /data/results/baseline --source-sha BASELINE_SHA \
  --conditions 'reference pixels; warm file cache; one process at a time'
```

Once extraction and compilation have finished, compare the candidate against
that reference and remeasure both binaries in interleaved order:

```sh
uv run tools/benchmark_corpus.py \
  --corpus /data/mcmap-corpus --binary /data/bin/candidate \
  --reference /data/results/baseline --reference-binary /data/bin/baseline \
  --time-binary /usr/bin/time --repeats 5 \
  --output /data/results/comparison --source-sha CANDIDATE_SHA \
  --conditions 'native Linux filesystem; warm file cache; idle machine'
```

The script runs one child at a time, checks source and context hashes, compares
every output pixel, checks output mtime, and records wall time and CLI elapsed
time. With GNU time it also records CPU time and peak RSS independently of the
Python driver. GNU time's CPU counters have 10 ms resolution. `--resume` requires
unchanged binaries and inputs; `--reference-only` supports incremental pixel
checks while additional archives are still being prepared. Such preparation-time
measurements are not performance acceptance evidence.

Report medians and tail latency by archive, stored-chunk density and visible area,
including the number of regions below 50 ms. Large compressed inputs and modded
entity data can dominate latency even when few surface pixels need shading.
Avoid presenting sparse regions as evidence for dense-region latency.

Use `--calculate-heights` with a separately generated reference to verify that
mode. The smaller `tools/benchmark_render.py` tool supports repeated comparisons
of representative regions and equivalent old/new-format copies.
