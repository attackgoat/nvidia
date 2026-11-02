# nvidia-streamline Changelog

## 0.2.0 (Unreleased)

- **Breaking:** Update the optional `vk-graph` adapter from 0.14.7 to 0.15.

Publication is pending the crates.io release of `vk-graph` 0.15 and completion
of the release gates.

## 0.1.0 (2026-09-05)

Published `nvidia-streamline` to crates.io. Initial extraction from private codebase.

- Vulkan interposer and DLSS Ray Reconstruction on x86-64 MSVC Windows.
- Default-enabled optional `vk-graph` 0.14.7 adapter and graph-independent Vulkan APIs.
- SDK-free documentation builds; no NVIDIA SDK payloads vendored in the crate.

The declared minimum Rust version is 1.95, including default-feature dependencies.
Native SDK components retain their upstream licenses separately from these
MIT OR Apache-2.0 bindings.
