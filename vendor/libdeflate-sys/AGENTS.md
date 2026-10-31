# Maintaining strict libdeflate

Read [source ownership and update rules](UPSTREAM.md) before editing this copy.

- Keep changes limited to preserving the streaming decoder's acceptance rules
  and the native build required by mcmap. Retain upstream attribution and licenses.
- The strict C implementation is required for normal builds. Do not enable
  dynamic linking to an unpatched system libdeflate.
- Use targeted differential tests against mcmap's actual `flate2/zlib-rs`
  backend. Cover malformed streams, exact output and original IO errors;
  checksum agreement alone does not establish equivalent validation.
- Preserve native runtime CPU dispatch and portable fallback implementations.
  Do not require host-specific instruction sets for distributed binaries.
- Compare release builds on identical compressed chunks before changing decode
  tables or the hot loop. Record local behavior and refresh evidence in
  `UPSTREAM.md` and the owning compatibility documentation.
