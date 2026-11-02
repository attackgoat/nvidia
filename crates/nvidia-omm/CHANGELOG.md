# nvidia-omm Changelog

## 0.1.0 (2026-09-05)

Published `nvidia-omm` to crates.io. Initial extraction from private codebase.

- Native CPU baking on x86-64 Linux/Windows and Intel/Apple silicon macOS.
- SDK-free documentation builds; no NVIDIA SDK payloads vendored in the crate.

The declared minimum Rust version is 1.95, including default-feature dependencies.
Native SDK components retain their upstream licenses separately from these
MIT OR Apache-2.0 bindings.
