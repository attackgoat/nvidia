# nvidia-sdk Changelog

This changelog covers SDK acquisition and staging. Integration release notes:

- [nvidia-dlss](https://github.com/attackgoat/nvidia/blob/main/crates/nvidia-dlss/CHANGELOG.md)
- [nvidia-nrd](https://github.com/attackgoat/nvidia/blob/main/crates/nvidia-nrd/CHANGELOG.md)
- [nvidia-omm](https://github.com/attackgoat/nvidia/blob/main/crates/nvidia-omm/CHANGELOG.md)
- [nvidia-streamline](https://github.com/attackgoat/nvidia/blob/main/crates/nvidia-streamline/CHANGELOG.md)

## 0.1.0 (2026-09-05)

Published `nvidia-sdk` to crates.io.

Initial extraction of the NVIDIA integrations from private codebase.

- Pinned, SHA-256-verified SDK acquisition, cache/offline support,
  explicit SDK overrides, source dependency assembly, and runtime staging.
- SHARC 1.8.3 shader-header acquisition on all targets.
- SDK-free documentation builds; no NVIDIA SDK payloads vendored in the crate.

The declared minimum Rust version is 1.95, including default-feature dependencies.
Native SDK components retain their upstream licenses separately from this
MIT OR Apache-2.0 crate.
