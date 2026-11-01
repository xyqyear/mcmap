# Community NBT fixtures

The 14 files are decompressed Java NBT resources from:

- [owengage/fastnbt](https://github.com/owengage/fastnbt), commit
  `60c7ee0de70c7ce6bfffc553fe372f834c3b32e9`.
- [azalea-rs/simdnbt](https://github.com/azalea-rs/simdnbt), commit
  `4d426efe074701bebec94b611ae90a78d07a11c3`.

`manifest.json` records each original path, SHA-256 and compression wrapper, plus
the decompressed input hashes and canonical reference digest. `fastnbt-LICENSE`
and `simdnbt-LICENSE` preserve the respective resource-source licenses. No parser
implementation from either repository is included.

The canonical reference was computed by an external tool using fastnbt 2.6.1,
mapping its values directly by tag into the representation in
`tests/support/mod.rs`. It does not parse through mcmap-nbt to produce expected
values. Regeneration requires explicit review of the reference and source pins;
do not refresh expected digests just because a candidate test fails.
