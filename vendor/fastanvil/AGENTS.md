# Maintaining fastanvil

This is an intentionally editable dependency, not generated or protected code.
Read [source ownership and update rules](UPSTREAM.md) before editing it.

- Implement changes here when they belong to the library and are clearer or
  more efficient here than in a mcmap workaround. Keep application policy in
  mcmap.
- Preserve upstream attribution and unrelated code. Document local behavior
  in `UPSTREAM.md` and its owning mcmap topic; do not rewrite upstream history.
- Add behavior tests for format changes, including old formats and malformed
  inputs. Use explicit test filters locally; complete CI runs after push.
- Performance changes must preserve pixels, source files and failure semantics.
  Compare optimized builds, matching output and thread counts, using repeated
  interleaved measurements. Include CPU and peak memory, not only wall time.
- `src/java/default_states_26_3.rs` is generated from the official Minecraft
  block report by `tools/update_default_states.py` at the mcmap repository root.
  Regenerate it; do not hand-edit block defaults.
