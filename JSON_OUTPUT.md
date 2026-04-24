# JSON Output Format

`mcmap` supports a machine-readable output mode enabled by the global
`--json` flag. Every subcommand accepts it:

```bash
mcmap --json render -r /world/region -p palette.json -o map.png
mcmap --json gen-palette modern -p 1.20.1.jar -o palette.json
```

## General shape

- Events are emitted as [newline-delimited JSON](https://jsonlines.org/) on
  **stdout**. Each line is exactly one JSON object terminated with `\n` and
  flushed immediately, so callers can split on `\n` and parse line-by-line.
- Human logs (`info!` / `warn!` / `error!`) go to **stderr** and are
  suppressed by default when `--json` is set. Re-enable them with
  `RUST_LOG=info` (or `debug`, etc.) if you want both streams.
- All lines share a `type` field as the discriminator. Four top-level types:
  - `progress` — repeatable. Intermediate phase or throughput update.
  - `region` — repeatable (render only). One entry per region processed.
  - `result` — terminal. Emitted exactly once on success, always the last
    line. Carries the final summary.
  - `error` — terminal. Emitted once on failure, always the last line.
    Process exits with code `1`.
- Parallelized commands (`render`) emit events from multiple rayon threads.
  Individual lines are atomic — `println!` / locked stdout writes don't
  interleave inside a single line — but event ordering across threads is
  **not** guaranteed. Rely on the fields inside each event rather than
  positional order.

### Common fields

| Field     | Always present? | Description                                                   |
|-----------|-----------------|---------------------------------------------------------------|
| `type`    | yes             | One of `progress`, `region`, `result`, `error`.               |
| `phase`   | `progress` only | Stable string identifying the phase. Phase names are per-command and listed below. |
| `message` | `error` only    | Human-readable error text. Not stable — do not pattern-match. |

Optional fields are only serialized when present (`null` is not emitted).

### Exit codes

| Code | Meaning                                                                           |
|------|-----------------------------------------------------------------------------------|
| `0`  | Terminal `result` event was emitted.                                              |
| `1`  | Terminal `error` event was emitted, or the CLI rejected the arguments (clap).     |

Argument-parse errors (from clap) are written to stderr in the usual textual
form and do **not** produce a JSON `error` event. Everything after arg
parsing does.

## `render`

Renders `.mca` regions to PNG. Two modes: combined (single output image) and
split (one PNG per region). The event stream reports per-region progress in
both.

### Event types

`progress` phases:

| `phase`           | Extra fields                                            |
|-------------------|---------------------------------------------------------|
| `palette_loaded`  | `elapsed_ms` (u128)                                     |
| `regions_listed`  | `count` (usize); `bounds` (combined mode only) — object `{xmin, xmax, zmin, zmax}` (i64) |
| `assembling`      | — (combined mode only; emitted once before the final image write) |

`region` events (one per region):

| Field      | Present?                               | Description                                                  |
|------------|----------------------------------------|--------------------------------------------------------------|
| `type`     | always                                 | `"region"`                                                   |
| `x`, `z`   | always                                 | Region coords (i64). These identify the region in the stream. |
| `status`   | always                                 | `"rendered"` / `"missing"` / `"error"`.                      |
| `output`   | split mode + `status:"rendered"`       | Path to the saved PNG (string).                              |
| `error`    | `status:"error"`                       | Error string.                                                |
| `warnings` | when non-empty                         | Array of strings. Currently covers `mtime_copy_failed: <err>` when `--preserve-mtime` is set and setting the PNG's mtime failed. |

`result` event (always last on success):

| Field            | Description                                                   |
|------------------|---------------------------------------------------------------|
| `mode`           | `"split"` or `"combined"`.                                    |
| `regions_saved`  | Regions successfully rendered (split) / successfully placed into the combined image. |
| `output`         | Destination path (combined: PNG path; split: directory path). |
| `elapsed_ms`     | Wall time from command start (u128).                          |

### Example (split mode)

```json
{"type":"progress","phase":"palette_loaded","elapsed_ms":94}
{"type":"progress","phase":"regions_listed","count":3}
{"type":"region","x":0,"z":0,"status":"rendered","output":"./tiles/r.0.0.png"}
{"type":"region","x":1,"z":0,"status":"missing"}
{"type":"region","x":0,"z":1,"status":"rendered","output":"./tiles/r.0.1.png","warnings":["mtime_copy_failed: Access is denied. (os error 5)"]}
{"type":"result","mode":"split","regions_saved":2,"output":"./tiles","elapsed_ms":3120}
```

## `analyze`

Scans regions for unknown blocks (blocks not present in the palette). 1.13+
only.

### Event types

`progress` phases:

| `phase`          | Extra fields                                            |
|------------------|---------------------------------------------------------|
| `palette_loaded` | `blockstates` (usize) — entries in the palette.         |
| `regions_listed` | `count` (usize).                                        |
| `scanning`       | `regions_scanned`, `chunks_scanned` (usize). Emitted every 10 regions. |

`result` event:

| Field              | Description                                                            |
|--------------------|------------------------------------------------------------------------|
| `regions_scanned`  | Regions actually opened (some may be absent / unreadable).             |
| `chunks_scanned`   | Chunks successfully decoded.                                           |
| `unique_blocks`    | Distinct block names found across all scanned chunks.                  |
| `unknown_blocks`   | Array of `{"name": "<namespace:id>", "count": <usize>}`, sorted by `count` descending. Blocks present in the palette are filtered out. |

Note: `unknown_blocks[].count` is always present in JSON mode regardless of
whether `--show-counts` is passed — that flag only affects the human-mode
console output.

### Example

```json
{"type":"progress","phase":"palette_loaded","blockstates":1873}
{"type":"progress","phase":"regions_listed","count":24}
{"type":"progress","phase":"scanning","regions_scanned":10,"chunks_scanned":4096}
{"type":"progress","phase":"scanning","regions_scanned":20,"chunks_scanned":8192}
{"type":"result","regions_scanned":24,"chunks_scanned":9804,"unique_blocks":412,"unknown_blocks":[{"name":"create:cogwheel","count":184},{"name":"mymod:weirdblock","count":3}]}
```

## `download-client`

Downloads a Minecraft client `.jar` from Mojang's piston-meta.

### Event types

`progress` phases:

| `phase`             | Extra fields                                                                |
|---------------------|-----------------------------------------------------------------------------|
| `manifest_fetched`  | —                                                                           |
| `version_resolved`  | `id` (string) — the concrete version id (e.g. `latest` → `"1.21.4"`).       |
| `download_info`     | `size` (u64), `sha1` (string) — expected jar size and hash.                 |
| `cache_hit`         | `path` (string) — tmp file path whose sha1 already matched; skips download. |
| `cache_miss`        | `path` (string) — tmp file will be (re-)written.                            |
| `downloading`       | `bytes` (u64), `total` (u64). Emitted at most every **500 ms**, plus a final tick at 100% when the body finishes. |
| `verified`          | — (size + sha1 checks passed).                                              |

`result` event:

| Field         | Description                                                    |
|---------------|----------------------------------------------------------------|
| `version`     | Resolved version id.                                           |
| `target`      | Final jar destination path.                                    |
| `bytes`       | Downloaded size in bytes (matches `download_info.size`).       |
| `sha1`        | Expected sha1 (also the observed sha1 since verification passed). |
| `move_method` | `"rename"` (atomic) or `"copy_fallback"` (fallback when rename failed — typically tmp and target were on different volumes). |

### Example

```json
{"type":"progress","phase":"manifest_fetched"}
{"type":"progress","phase":"version_resolved","id":"1.21.4"}
{"type":"progress","phase":"download_info","size":28374651,"sha1":"f1e2...ab"}
{"type":"progress","phase":"cache_miss","path":"/tmp/mcmap-client-1.21.4.jar.part"}
{"type":"progress","phase":"downloading","bytes":4194304,"total":28374651}
{"type":"progress","phase":"downloading","bytes":12582912,"total":28374651}
{"type":"progress","phase":"downloading","bytes":28374651,"total":28374651}
{"type":"progress","phase":"verified"}
{"type":"result","version":"1.21.4","target":"./client.jar","bytes":28374651,"sha1":"f1e2...ab","move_method":"rename"}
```

## `gen-palette`

All three subcommands (`modern`, `legacy`, `forge112`) share the same event
skeleton but differ in which events they emit and in the shape of `counters`.

### Common events (all three)

`progress` phases:

| `phase`             | Extra fields                                                              |
|---------------------|---------------------------------------------------------------------------|
| `registry_loaded`   | `legacy`/`forge112` only. `blocks` (usize); `legacy` also carries `items` (usize). |
| `pack_loaded`       | `path` (string); `index` (usize, 1-based) and `total` (usize) — current pack position and total number of top-level packs discovered, so each event is self-contained for "N of M" progress rendering; `blockstates_added`, `models_added`, `textures_added` (usize); `error` (string, optional — present when the archive failed to load, in which case the added-counts are zero). One event per input pack in the order they were provided. Nested Forge jarjar entries do **not** produce their own events — their contributions are folded into the parent's added-counts — so `total` reflects top-level archives only. For `legacy`, `blockstates_added` and `models_added` are always `0` because 1.7.10 packs don't ship those JSONs. |
| `packs_done`        | `pack_count` (usize) and totals. `modern`/`forge112`: `blockstates`, `models`, `textures`. `legacy`: `textures` only. |
| `resolved`          | `counters` — shape depends on subcommand (see below). `modern` also carries `failed` (usize). |
| `overrides_applied` | `count` (usize). Only emitted when `--overrides` was passed.              |

### `result` event

| Field       | All | Description                                                       |
|-------------|-----|-------------------------------------------------------------------|
| `output`    | yes | Written palette.json path.                                        |
| `entries`   | yes | Total palette entries written.                                    |
| `counters`  | yes | Same shape as the `resolved` event.                               |
| `failed`    | `modern` only | Number of blockstates that failed to resolve through any tier. |

### `modern` counters

```json
{
  "rendered":           <usize>,   // top-face render succeeded
  "side_fallback":      <usize>,   // raw-model fallback hit (multipart / side face)
  "particle":           <usize>,   // particle-texture fallback
  "any_texture":        <usize>,   // any-texture-on-model fallback
  "regex_mapped":       <usize>,   // generic regex rewrite matched (note: field was `mapped` internally; renamed for stability)
  "probed":             <usize>,   // direct block/<name> texture probe
  "substring":          <usize>,   // substring bridge (custom state mappers)
  "generic_blockstate": <usize>    // generic blockstate bridge (dynamic registries)
}
```

### `legacy` counters

```json
{
  "vanilla":      <usize>,  // emitted from the curated (name, meta) table
  "modded_exact": <usize>,  // modded block matched a file exactly
  "modded_fuzzy": <usize>,  // modded block matched via case-insensitive / prefix-strip / substring
  "fallback":    <usize>,   // modded block got the gray fallback
  "missing":     <usize>,   // vanilla block's texture file wasn't found
  "malformed":   <usize>    // registry entry wasn't `<ns>:<name>`
}
```

### `forge112` counters

Forge 1.12.2 runs the full modern resolver on modded blocks plus the legacy
curated table for vanilla, so it reports **both** sets, nested:

```json
{
  "classification": {
    "vanilla_table":     <usize>,  // resolved via the shared 1.x table
    "modded_resolved":   <usize>,  // resolved via the modern resolver
    "fallback_gray":     <usize>,  // neither succeeded
    "skipped":           <usize>   // registry entry was empty / malformed
  },
  "resolver": {
    // same shape as `modern` counters — applies to the modded path
    "rendered":           <usize>,
    "side_fallback":      <usize>,
    "particle":           <usize>,
    "any_texture":        <usize>,
    "regex_mapped":       <usize>,
    "probed":             <usize>,
    "substring":          <usize>,
    "generic_blockstate": <usize>
  }
}
```

### Example (`gen-palette modern`)

```json
{"type":"progress","phase":"pack_loaded","path":"./create-0.5.jar","index":1,"total":2,"blockstates_added":412,"models_added":910,"textures_added":688}
{"type":"progress","phase":"pack_loaded","path":"./1.20.1.jar","index":2,"total":2,"blockstates_added":801,"models_added":1540,"textures_added":1324}
{"type":"progress","phase":"packs_done","pack_count":2,"blockstates":1213,"models":2450,"textures":2012}
{"type":"progress","phase":"resolved","counters":{"rendered":1020,"side_fallback":110,"particle":18,"any_texture":34,"regex_mapped":21,"probed":5,"substring":2,"generic_blockstate":0},"failed":3}
{"type":"result","output":"./palette.json","entries":1213,"failed":3,"counters":{"rendered":1020,"side_fallback":110,"particle":18,"any_texture":34,"regex_mapped":21,"probed":5,"substring":2,"generic_blockstate":0}}
```

## Error event

Any failure after argument parsing terminates the stream with:

```json
{"type":"error","message":"<human-readable error text>"}
```

The exit code is `1`. No `result` event is emitted in this case. The message
text is not stable; do not pattern-match against it — check that
`type == "error"` and handle the failure generically. The partial progress
events emitted before the error are still valid and can be used to report
what the command had done before it failed.

## Stability

Field names and `phase` identifiers documented here are intended to be
stable. New `progress` phases may be added in future versions — parsers
should tolerate unknown `phase` values by ignoring them, and ignore unknown
fields on known events.
