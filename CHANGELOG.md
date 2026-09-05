# Changelog

## 0.1.0 (Unreleased)

Initial extraction of the NVIDIA integrations from private codebase.

- `nvidia-sdk`: pinned, SHA-256-verified SDK acquisition, cache/offline support,
  explicit SDK overrides, source dependency assembly, and runtime staging.
- `nvidia-dlss`: Vulkan NGX DLSS Ray Reconstruction on x86-64 GNU Linux.
- `nvidia-streamline`: Vulkan interposer and DLSS Ray Reconstruction on
  x86-64 MSVC Windows.
- `nvidia-nrd`: NRD RELAX SH on x86-64 Linux/Windows and Apple silicon macOS.
- `nvidia-omm`: native CPU baking on x86-64 Linux/Windows and Intel/Apple
  silicon macOS.
- SHARC 1.8.3 shader-header acquisition on all targets through `nvidia-sdk`.
- Default-enabled optional `vk-graph` adapters and graph-independent Vulkan APIs.
- SDK-free documentation builds; no NVIDIA SDK payloads vendored in these crates.

The declared minimum Rust version is 1.95, including default-feature dependencies.
Native SDK components retain their upstream licenses separately from these
MIT OR Apache-2.0 bindings.
