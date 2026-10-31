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

`anvil/modern/codec.rs` first uses fastanvil's native modern decoder. When that
fails on a chunk with `sections`, it normalizes block palettes in memory and
decodes the modern schema directly. Other chunk layouts retain fastanvil's older
format decoders. Region files and their NBT are never rewritten by `render` or
`analyze`.

Explicit 26.3 properties override the embedded vanilla defaults. The defaults
come from the official 26.3 block report, rather than inferring them from model
variant order or property names. Unknown modded blocks retain their explicit
properties and the existing palette fallback; their omitted default properties
cannot be reconstructed from a vanilla report.

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
  generated/reports/blocks.json src/anvil/modern/default_states_26_3.json
```

The extractor requires exactly one default state per block and stores only blocks
with default properties. The 26.3 report contains 1,286 blocks; 884 have default
properties. The generated table is deterministic and loaded once per process.

## Verification

`cargo test --locked --test minecraft_26_3` checks actual CLI output pixels for
old/new chunk formats, default and explicit properties, packed palette indices,
block analysis, input preservation, malformed data and legitimate empty regions.
The Integration tests workflow runs these checks before its real-server matrix,
including pinned 26.2 and the latest vanilla release.

Local production-data verification uses copied MCA files and a copied color
palette. Those private world files are not repository fixtures. Compare source
digests before/after, region outcomes, decoded pixels and runtime; a zero exit
code or the existence of a PNG alone does not establish successful rendering.
