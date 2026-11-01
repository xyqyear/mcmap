# mcmap NBT

This private workspace crate owns Java NBT decoding for mcmap and its maintained
fastanvil copy. It has no fastnbt dependency, including in tests. Minecraft chunk
schemas and palette caching belong to fastanvil, not this crate. The public
surface serves mcmap's callers; it is not a replacement for every upstream
fastnbt API.

## Structure

| Module | Responsibility |
| --- | --- |
| `cursor`, `skip` | Checked big-endian reads and structural skipping |
| `de`, `access` | Serde deserialization and compound/list/array access |
| `arrays`, `borrow` | Owned numeric arrays and temporary borrowed strings |
| `value` | Untyped NBT values, preserving all 12 tag kinds |
| `ser`, `write` | Serde-to-value conversion and Java NBT encoding |
| `macros` | Small fixture/value construction helper |

Keep wire-format rules here and Minecraft version-specific rules in fastanvil.
Array readers convert borrowed input bytes directly into the final owned vector.
The historical internal array identifiers remain reserved so real compounds
cannot impersonate arrays. They do not represent a dependency on another parser.
Strings use `cesu8` for Java modified UTF-8, including NUL and surrogate pairs.
The crate forbids unsafe Rust.

## Input contract

The root must be a named compound. Its name is skipped; callers receive its
payload. Trailing bytes after the decoded root remain tolerated. Numeric
coercions use Serde's checked conversions. Lists and the three array tags remain
distinct, duplicate struct fields fail, and duplicate dynamic compound keys keep
their last value. Returned arrays and dynamic values own their data.

Unknown fields are skipped without allocating their contents. At 64 nested
containers the skipper switches from recursion to an explicit stack. Ignored
strings do not require character decoding; ignored nonpositive lists retain the
previous empty-list behavior. Materialized lists reject negative lengths and
lengths above 100 million. Materialized containers have a 512-level nesting
limit, returning an error instead of overflowing the stack. This limit does not
apply to skipped subtrees.

Writing supports the application's compounds, homogeneous lists, arrays, numeric
types, strings, optional fields, newtype wrappers and unit enum names. `None`
fields are omitted; null list elements, heterogeneous lists and enum payloads
are rejected. The writer builds an intermediate owned value because writing is
outside the rendering hot path. Do not generalize the serializer without an
actual caller and a wire-format regression test.

## Correctness

`tests/contract.rs` uses independently assembled wire bytes to cover types,
endianness, truncation, malformed lengths, floating-point bits, Unicode, array
ownership, duplicate fields, deep skipping and writer errors.

`tests/community.rs` checks 14 pinned community resources against canonical
reference digests generated with fastnbt 2.6.1 outside this workspace, and checks
read/write round trips. The canonical representation includes every tag, array
element and floating-point bit; only compound key ordering is normalized.
`tests/fixtures/manifest.json` records source repositories, pinned commits,
original paths, wrapper removal and hashes. Both source licenses are retained.

```sh
cargo test --locked -p mcmap-nbt --test contract --test community
```

Large private real-world corpora and performance runs stay outside Git and CI.
See [performance](../../PERFORMANCE.md) for the rendering comparison contract.
