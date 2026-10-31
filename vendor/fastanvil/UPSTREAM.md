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
cache invalidation and world-management orchestration in mcmap. Changes to a
different dependency, including fastnbt, require their own evidence and scope.

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

See [Minecraft compatibility](../../MINECRAFT_COMPATIBILITY.md) for supported
formats, default-state provenance and the decoding/performance contract.
