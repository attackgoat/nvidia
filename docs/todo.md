# Review TODO

- [x] Fix the required CI Clippy job without relaxing the strict workspace lint policy. Added error documentation and annotations and corrected casts and other reported warnings.
- [x] Expose initialization-time `DlssRrConfig` for HDR/LDR color, render/output-resolution primary motion, and reversed hardware, forward hardware, or linear view-space depth. Defaults retain the existing conventions; changing configuration requires shutdown and reinitialization.
- [x] Validate DLSS input `SAMPLED` and output `STORAGE` usage, layouts, and configured image dimensions before evaluation in Rust and C++.
- [x] Document per-axis zero motion-scale fallback to unity. Native regression tests run the real SDK helpers and inspect the effective NGX parameter values, including negative zero.
- [x] Accept minimally sized pitched OMM mip buffers using checked `(height - 1) * row_pitch + width` arithmetic. Added tight-final-row, truncation, overflow, and native bake tests.
- [x] Clarify that NRD pre-FFI errors consume no slots and failures after `NewFrame` can consume slots. Retain a conservative count-every-call retirement rule because errors do not reliably identify advancement; call counts must not be used to infer native slot indices.

## Current Verification

- Linux native workspace tests, all features: 40 passed, 1 intentionally ignored download test.
- `cargo fmt --all --check`: passed.
- `git diff --check`: passed.
- Strict workspace Clippy: passed with all features for native and forced-stub builds, and with no default features for forced stubs.
- Workspace documentation with warnings denied: passed.
- DLSS standalone C++ regression: passed for all 12 configuration combinations and 120 evaluations.
- Targeted native/stub crate tests and strict Clippy checks passed across feature variants.
- GPU execution and non-Linux native C++ builds remain unverified. Windows Streamline Rust checks do not establish C++ linking or runtime correctness.
