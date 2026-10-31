# Minecraft chunk compatibility

## Minecraft Java 26.3

Minecraft 26.3 changes block-state serialization inside chunk section palettes:

| State | NBT representation |
| --- | --- |
| Earlier chunk formats | `{Name: "minecraft:oak_log", Properties: {axis: "y"}}` |
| 26.3 non-default state | `{id: "minecraft:oak_log", properties: {axis: "x"}}` |
| 26.3 default state | `"minecraft:oak_log"` |
| Default state inside a compound list | `{"": "minecraft:oak_log"}` |

The last form is NBT's heterogeneous-list wrapper. A string denotes the block's
default state, including properties omitted from the save. It does not mean that
the block has no properties. Existing chunks can retain the earlier form in a
world running 26.3.

The change is documented in the official
[26.3 Snapshot 7 notes](https://www.minecraft.net/en-us/article/minecraft-26-3-snapshot-7).
The official 26.2 and 26.3 server JARs also establish the codec difference:

- `StateHolder.codec` uses `Name` / `Properties` in 26.2 and `id` / `properties`
  in 26.3.
- `BlockState.CODEC` in 26.3 combines the block registry's name codec with
  `FULL_CODEC`. States equal to `Block.defaultBlockState()` use the name form.
- Official server JAR SHA-1: 26.2
  `823e2250d24b3ddac457a60c92a6a941943fcd6a`; 26.3
  `33680f5f2ac32864d6d7cf5e56a705fdb3e05f4c`.

An independent implementation is BlueMap's
[BlockStateDeserializer](https://github.com/BlueMap-Minecraft/BlueMap/blob/96ae320d1d39581c10058d8222f2290f2fcb5333/core/src/main/java/de/bluecolored/bluemap/core/world/mca/data/BlockStateDeserializer.java),
which handles both field spellings, strings, the empty-name wrapper and default
state resolution.

## Reading and rendering

The maintained [fastanvil dependency](vendor/fastanvil/UPSTREAM.md) reads both
block-state formats directly through its `Block` deserializer. Modern chunks
need one typed NBT decode; unknown fields are skipped without constructing a
whole-chunk `Value` tree. Palette names and property strings borrow from the
input during construction; known default states borrow compiled names and
encoded descriptions. Other chunk layouts retain
fastanvil's older format decoders. Region files and their NBT are never rewritten
by `render` or `analyze`.

Explicit 26.3 properties override the embedded vanilla defaults. The defaults
come from the official 26.3 block report, rather than inferring them from model
variant order or property names. Unknown modded blocks retain their explicit
properties and the existing palette fallback; their omitted default properties
cannot be reconstructed from a vanilla report.
Default-only states borrow a compiled template; states with explicit overrides
use a small property vector. A read-only hash index over the finite official
default-state table is initialized once. Input-derived states are not retained
in a global cache, and custom palette colors remain specific to each invocation.

A malformed chunk fails its region and emits a `region` event with `status:error`.
Split rendering does not publish a PNG for that region. If no region succeeds,
both output modes emit a terminal error and exit nonzero. Empty regions and
successfully decoded air-only chunks are valid transparent maps. In a partially
successful multi-region run, callers must still inspect each region's status.

Replacing the binary does not invalidate PNG files cached by an external caller.
An application using source mtime alone must separately invalidate affected old
tiles; mcmap does not edit application caches automatically.

## Refreshing vanilla defaults

Run the official server data generator in a disposable directory using the Java
version declared in that release's Mojang version manifest:

```sh
java -DbundlerMainClass=net.minecraft.data.Main -jar server.jar --reports --output generated
uv run --no-project python tools/update_default_states.py \
  generated/reports/blocks.json vendor/fastanvil/src/java/default_states_26_3.rs
```

The extractor requires exactly one default state per block. The 26.3 report
contains 1,286 blocks; 884 have default properties. The deterministic Rust
templates are compiled into the binary; the runtime lookup index references
those templates without parsing JSON or copying their strings. Explicit
properties are merged with static defaults only for the modern representations;
legacy `Name` entries retain their stored property semantics.

## Verification

`cargo test --locked --test minecraft_26_3` checks actual CLI output pixels for
old/new chunk formats, default and explicit properties, packed palette indices,
block analysis, input preservation, malformed data and legitimate empty regions.
The Integration tests workflow runs these checks before its real-server matrix,
including pinned 26.2 and the latest vanilla release. It also runs the vendored
library's complete unit suite. Local library checks use the explicit filter
`cargo test --locked -p fastanvil --lib java::block::tests`.
Isolated test servers explicitly disable their whitelist so synthetic clients
can join even though [26.3 Snapshot 2](https://www.minecraft.net/en-us/article/minecraft-26-3-snapshot-2)
changed the game's default to enabled. Player-presence and pruning assertions
still require a real client connection and saved inhabited-time data.
The synthetic client uses protocol 777's packet IDs from the official 26.3
`reports/packets.json`; known-pack selection and the clientbound play login,
keep-alive and position packets differ from 26.2. Unknown protocol versions fail
explicitly instead of guessing that a newer release shares an older layout.
The 26.3 teleport confirmation carries position and rotation as well as its ID,
as specified by the official server's `ServerboundAcceptTeleportationPacket`
codec. The client also sends tick-end packets and fails the test on an unexpected
disconnect, so a vanished player cannot masquerade as low inhabited time.

Local production-data verification uses copied MCA files and a copied color
palette. Those private world files are not repository fixtures. Compare source
digests before/after, region outcomes, decoded pixels and runtime; a zero exit
code or the existence of a PNG alone does not establish successful rendering.

`tools/benchmark_render.py` requires Linux and GNU time (`/usr/bin/time`, or
`--time-binary`). It runs release binaries in randomized,
interleaved order, with a warm-up for each case/thread count. Each measured
process records wall time, CPU time and peak RSS. GNU time measures the native
process's memory separately from the Python driver; its CPU times have 10 ms
resolution. Pixel hashes must match between
every case, and input hashes must remain unchanged. Use equivalent copies of
the same terrain in old and 26.3 formats, the same palette and identical compiler
and target settings; a fast blank image from an incompatible binary is not a
valid baseline. Keep world copies and raw timing reports outside Git.

```sh
uv run tools/benchmark_render.py --palette /tmp/bench/palette.json \
  --case baseline /tmp/bench/baseline /tmp/bench/old-format \
  --case candidate-old /tmp/bench/candidate /tmp/bench/old-format \
  --case candidate-new /tmp/bench/candidate /tmp/bench/26.3 \
  --threads 1 2 --repeats 15 --output /tmp/bench/results
```

### Release performance evidence

The 2026-10-31 comparison used rustc 1.90.0, the Linux GNU target, the same
release/LTO settings, and two copied regions containing 1,839 stored chunks
(959 fully generated). Each case ran 15 measured times per thread count in
interleaved order after warming the files. Old-format copies represent the same
block states, including explicit vanilla defaults. All rendered RGBA bytes
matched across every case and source hashes remained unchanged.

| Binary and input | 1 thread median | 2 threads median |
| --- | ---: | ---: |
| 0.8.4, equivalent old format | 175.9 ms | 125.0 ms |
| 0.8.5, equivalent old format | 175.6 ms | 125.6 ms |
| 0.8.5, original 26.3 | 280.1 ms | 189.6 ms |
| 0.8.6, equivalent old format | 167.1 ms | 119.4 ms |
| 0.8.6, original 26.3 | 174.4 ms | 124.7 ms |

For the original 26.3 files, median CPU time was 270 ms with 0.8.5 and 160 ms
with 0.8.6 for either thread count (GNU time's 10 ms resolution). Median peak
RSS was 24,280 / 28,912 KiB with 0.8.5 and 21,152 / 28,312 KiB with 0.8.6
(one / two threads). Region iteration order and allocator behavior affect RSS;
the raw benchmark includes completion order for that reason.

This sample shows parity with the pre-compatibility release's successful
rendering baseline and a 34–38% reduction versus 0.8.5 on 26.3. The optimized
old-format path itself is faster: 26.3 remains about 4–5% slower than that path
within 0.8.6. These are measured end-to-end results, not a guarantee of zero
format-dependent cost on every world, platform or input distribution. Do not
compare these absolute GNU-target timings to earlier musl-target measurements.
