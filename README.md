# nvidia-rs

[![CI](https://github.com/attackgoat/nvidia/actions/workflows/ci.yml/badge.svg)](https://github.com/attackgoat/nvidia/actions/workflows/ci.yml)
[![Rust 1.95+](https://img.shields.io/badge/rust-1.95%2B-orange?logo=rust)](https://www.rust-lang.org/tools/install)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)
[![GitHub](https://img.shields.io/badge/GitHub-attackgoat%2Fnvidia-181717?logo=github)](https://github.com/attackgoat/nvidia)

Rust integrations for selected NVIDIA graphics SDKs.

| Crate | crates.io | API Documentation |
| --- | --- | --- |
| SDK acquisition and SHARC headers | [![nvidia-sdk](https://img.shields.io/crates/v/nvidia-sdk.svg)](https://crates.io/crates/nvidia-sdk) | [![docs.rs](https://docs.rs/nvidia-sdk/badge.svg)](https://docs.rs/nvidia-sdk) |
| DLSS / NGX | [![nvidia-dlss](https://img.shields.io/crates/v/nvidia-dlss.svg)](https://crates.io/crates/nvidia-dlss) | [![docs.rs](https://docs.rs/nvidia-dlss/badge.svg)](https://docs.rs/nvidia-dlss) |
| NRD | [![nvidia-nrd](https://img.shields.io/crates/v/nvidia-nrd.svg)](https://crates.io/crates/nvidia-nrd) | [![docs.rs](https://docs.rs/nvidia-nrd/badge.svg)](https://docs.rs/nvidia-nrd) |
| OMM CPU baker | [![nvidia-omm](https://img.shields.io/crates/v/nvidia-omm.svg)](https://crates.io/crates/nvidia-omm) | [![docs.rs](https://docs.rs/nvidia-omm/badge.svg)](https://docs.rs/nvidia-omm) |
| Streamline | [![nvidia-streamline](https://img.shields.io/crates/v/nvidia-streamline.svg)](https://crates.io/crates/nvidia-streamline) | [![docs.rs](https://docs.rs/nvidia-streamline/badge.svg)](https://docs.rs/nvidia-streamline) |

See the [changelog](https://github.com/attackgoat/nvidia/blob/main/CHANGELOG.md)
and [release checklist](https://github.com/attackgoat/nvidia/blob/main/RELEASE.md).

> [!NOTE]
> This project is independently maintained. It is not affiliated with, sponsored
> by, or endorsed by NVIDIA Corporation.
>
> NVIDIA and the names of its products are trademarks of NVIDIA Corporation.

The repository contains no NVIDIA SDK payloads. Native builds use an explicitly
configured SDK or a local cache. They can download a pinned official release
when neither is available.

Downloaded SDKs retain their upstream licenses. This project's open-source
license does not cover them.

## Status

The crates share one SDK resolver and command-line interface. Each integration
has its own platform requirements.

| Integration | Native Support | SDK Inputs |
| --- | --- | --- |
| DLSS Ray Reconstruction through Vulkan NGX | x86-64 GNU Linux | Pinned NGX/DLSSD SDK |
| Streamline | x86-64 MSVC Windows | Pinned Streamline SDK |
| NRD RELAX SH | x86-64 Linux/Windows; Apple silicon macOS | Pinned source archives and dependencies |
| OMM CPU baker | x86-64 Linux/Windows; Intel/Apple silicon macOS | Shared libraries on Linux/Windows; pinned sources on macOS |
| SHARC 1.8.3 | Shader headers on all targets | Pinned upstream source archive |

Unsupported native targets use stub backends. See
[`SOURCE-SDKS.md`](https://github.com/attackgoat/nvidia/blob/main/crates/nvidia-sdk/SOURCE-SDKS.md)
for exact target rules, native toolchains, and source dependency requirements.

### SHARC Headers

SHARC is a shader-only SDK. It does not introduce a renderer abstraction or a
native backend.

The resolver returns the official SDK root. Headers are under `include/`, and
the license is `License.md`.

> [!TIP]
> Use `resolved.path.join("include")` as the shader include directory.
> Point `SHARC_SDK` at the SDK root, not at `include/`.

The [SHARC source notes](https://github.com/attackgoat/nvidia/blob/main/crates/nvidia-sdk/SOURCE-SDKS.md#sharc-183)
record the verified commit and archive pin. They also provide the exact layout
and prefetch commands. These inputs are acquired, not vendored here.

## Renderer Integrations

`nvidia-dlss`, `nvidia-nrd`, and `nvidia-streamline` enable `vk-graph` by default.
The feature is optional. Disable default features to use the underlying
`ash`/Vulkan APIs without a `vk-graph` dependency.

OMM and the SDK resolver are renderer-independent. Neither has a `vk-graph`
feature.

The default dependency graph requires **Rust 1.95 or newer**.

### Graph-Free Configuration

Use this crates.io configuration for graph-free Vulkan integration:

```toml
[dependencies]
nvidia-dlss = { version = "0.1.0", default-features = false, features = ["dlss-release-runtime"] }
nvidia-nrd = { version = "0.1.1", default-features = false, features = ["native"] }
nvidia-streamline = { version = "0.1.0", default-features = false }
```

Omit `default-features = false` to use the default `vk-graph` adapters.

For the CPU baker, use
`nvidia-omm = { version = "0.1.0", features = ["native"] }`.
For SDK acquisition alone, use `nvidia-sdk = "0.1.0"`.

### Raw Vulkan APIs

| Integration | Entry Points |
| --- | --- |
| DLSS | `VulkanInitInfo`, `Ngx::initialize_raw`, `Ngx::device_extensions_raw` |
| NRD | `Nrd::from_raw` |
| Streamline on Windows | `Streamline::create_proxy_ash_instance`, returning `ProxyInstanceOwner` |

The Streamline entry point creates an instance through its interposer. It does
not attach Streamline to arbitrary externally created devices.

> [!WARNING]
> Raw Vulkan APIs are unsafe. Callers must uphold the documented handle
> lifetimes and GPU-safe shutdown order.
>
> Graph adapters also require unsafe calls where ownership cannot prove device
> provenance, enabled extensions, queue configuration, or creation-pointer validity.

The default graph adapters retain their device and instance ownership behavior.

### Feature Selection

Renderer features do not control native SDK support or acquisition.

| Configuration | Behavior |
| --- | --- |
| NRD without default features | Enable `native` to retain the native backend. |
| DLSS without `dlss-release-runtime` | Uses the development runtime. |
| Unsupported native platform | Uses a stub regardless of renderer features. |

Contributions for additional renderer adapters are welcome. Keep adapters
optional and the core APIs independent of any particular renderer.

Additional graphics-API backends are also welcome as implemented and tested
changes. The current Vulkan bindings do not imply support for those APIs.

## SDK Resolution

Resolution uses the first available source:

1. The SDK-specific variable: `DLSS_SDK`, `NRD_SDK`, `OMM_SDK`, `SHARC_SDK`, or
   `STREAMLINE_SDK`.
2. `NVIDIA_SDK_ROOT/<sdk>/<version>`.
3. The verified user cache.
4. A pinned official download.

| Setting | Effect |
| --- | --- |
| `NVIDIA_SDK_CACHE` | Overrides the cache directory. |
| `NVIDIA_SDK_OFFLINE=1` | Prohibits SDK downloads. |
| Cargo offline mode | Also prohibits SDK downloads. |

Explicitly configured SDK directories are never replaced or modified.

> [!IMPORTANT]
> Downloading or using an SDK may accept additional NVIDIA terms. Review the
> URL and license reported by the resolver before enabling native integrations.

## License

Project-authored Rust and native bridge code is dual-licensed. Choose either:

- [Apache License, Version 2.0](https://github.com/attackgoat/nvidia/blob/main/LICENSE-APACHE)
- [MIT License](https://github.com/attackgoat/nvidia/blob/main/LICENSE-MIT)

NVIDIA SDKs and other downloaded third-party components retain their own
licenses.
