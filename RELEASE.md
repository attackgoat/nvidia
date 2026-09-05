# Release Readiness

## Approval Gate

> [!IMPORTANT]
> This checklist does not authorize a release. Obtain explicit user approval for
> the exact versions and release actions before publishing, pushing, or tagging.
> Do not commit, tag, push, or publish as part of readiness checks.

CI never publishes automatically. Stop on failures and record blockers, not
assumed passes.

## SDK-Free Checks

Run from the workspace root (POSIX shell):

```sh
export DOCS_RS=1
export NVIDIA_SDK_OFFLINE=1
export RUSTDOCFLAGS='-D warnings'
cargo doc --workspace --no-deps
cargo doc --workspace --no-deps --no-default-features
cargo doc --workspace --no-deps --no-default-features --features nvidia-nrd/native,nvidia-omm/native
```

`DOCS_RS` presence selects existing stubs, even with `native` enabled. This happens
before SDK resolution, native compilation, or runtime staging.

Unset `DOCS_RS` to restore native behavior. Setting it to `0` still selects stubs.
Build scripts track changes to this variable and retain their `rustc-check-cfg`
declarations.

> [!WARNING]
> These commands simulate the docs.rs build-script path, not the complete docs.rs
> service environment. They provide **no native test coverage**.
> Do not use simulation success as native release evidence.

Automatic Linux/Windows CI runs docs, Clippy, and resolver/stub tests under these
same guards. It covers default, no-default, and native-enabled/no-vk-graph features.

SDK acquisition is offline, but Cargo registry dependency downloads are still
allowed. No SDK payloads, CMake, DXC, GPU, or driver are needed for this simulation.
Ignored network tests are not run.

## Native Checks

Automatic CI runs a separate native job without `DOCS_RS` on these native hosts:

| Host | Native Coverage |
| --- | --- |
| x86-64 GNU Linux (`ubuntu-24.04`) | DLSS default-feature check (release runtime/graph adapter), no-default-feature unit tests (development runtime); OMM CPU bake and layout tests |
| x86-64 MSVC Windows (`windows-2022`) | Streamline default-feature check (graph adapter), no-default-feature layout/API unit tests; OMM CPU bake and layout tests |
| Intel macOS (`macos-15-intel`) | OMM static source build, CPU bake and layout tests |
| Apple silicon macOS (`macos-15`) | OMM static source build, CPU bake and layout tests |

These jobs acquire pinned SDKs and use the hosted native C++ toolchains. OMM
tests enable `native` without default features. DLSS/Streamline tests do not
initialize a GPU or validate rendering; OMM executes the real CPU baker.
Workflow configuration is not evidence of a passing run: record CI results for
the release revision before marking these targets validated.

NRD native builds/tests remain an explicit **manual release gate** on x86-64
Linux/Windows and Apple silicon macOS. Automatic CI does not provision its
required CMake 3.30+ and SPIR-V-capable Vulkan SDK DXC, and deliberately avoids
native workspace-wide commands that would pull in NRD. Intel macOS is an NRD stub.

> [!WARNING]
> The commands below are native build/FFI/CPU smoke checks, not GPU rendering tests.
> Validate DLSS/NRD rendering separately with a supported GPU, driver, and consuming
> app. Streamline compilation is not runtime validation.

<details>
<summary>x86-64 GNU Linux: toolchain, SDK verification, and native checks</summary>

On an x86-64 GNU Linux host, install a native C++ compiler, CMake 3.30+, and
Vulkan SDK DXC on PATH first. Review `crates/nvidia-sdk/SOURCE-SDKS.md` and the
pinned SDK licenses. Then run:

```sh
unset DOCS_RS NVIDIA_SDK_OFFLINE CARGO_NET_OFFLINE
cmake --version
dxc --version
cargo run -p nvidia-sdk -- verify dlss --target x86_64-unknown-linux-gnu
cargo run -p nvidia-sdk -- verify nrd --target x86_64-unknown-linux-gnu
cargo run -p nvidia-sdk -- verify omm --target x86_64-unknown-linux-gnu
export NVIDIA_SDK_OFFLINE=1
cargo test -p nvidia-dlss --no-default-features native_extension_output_matches_cpp
cargo test -p nvidia-nrd --no-default-features --features native ffi_layout_matches_native_bridge
cargo test -p nvidia-omm --no-default-features --features native native_baker_returns_owned_special_indices
cargo check --workspace --all-targets --features nvidia-omm/native
```

</details>

<details>
<summary>x86-64 Windows: SDK verification and native checks</summary>

On x86-64 Windows use a Visual Studio developer PowerShell with CMake 3.30+
and `VULKAN_SDK` pointing to a Vulkan SDK containing `Bin/dxc.exe`:

```powershell
Remove-Item Env:DOCS_RS, Env:NVIDIA_SDK_OFFLINE, Env:CARGO_NET_OFFLINE -ErrorAction SilentlyContinue
cargo run -p nvidia-sdk -- verify streamline --target x86_64-pc-windows-msvc
cargo run -p nvidia-sdk -- verify nrd --target x86_64-pc-windows-msvc
cargo run -p nvidia-sdk -- verify omm --target x86_64-pc-windows-msvc
$env:NVIDIA_SDK_OFFLINE = '1'
cargo check -p nvidia-streamline --all-targets
cargo test -p nvidia-nrd --no-default-features --features native ffi_layout_matches_native_bridge
cargo test -p nvidia-omm --no-default-features --features native native_baker_returns_owned_special_indices
```

</details>

macOS native OMM requires Xcode command-line tools and C++20. Apple silicon NRD
additionally requires CMake 3.30+ and DXC.

- [ ] On each supported NRD native host, with `DOCS_RS` unset and the required
  toolchain installed, run `cargo test -p nvidia-nrd --no-default-features --features native ffi_layout_matches_native_bridge` and record the result.
- [ ] Record supported-host results and remaining platform/runtime gaps.
- [ ] Validate rendering separately from native build/FFI/CPU smoke checks.

## Package Checklist

Use a symlink-capable checkout as required by `CONTRIBUTING.md`. Complete the
following gates in order.

- [ ] Verify that each crate's license links resolve to the root texts. Generated
  `.crate` archives must contain the full license contents as regular files, not
  link targets.
- [ ] Have the manifest owners confirm publishable metadata, licenses/notices,
  package includes, docs.rs metadata, and versioned path dependencies on
  `nvidia-sdk`. Do not edit manifests implicitly.
- [ ] Ensure all vk-graph dependencies use crates.io **0.14.7**. Do not use a git
  branch, local patch, or sibling checkout.
- [ ] Review `git status --short` and `git diff`. Release verification requires a
  reviewed clean checkout. Do not bypass this with `--allow-dirty`.
- [ ] Inspect package contents in dependency order:

```sh
cargo package -p nvidia-sdk --list
cargo package -p nvidia-dlss --list
cargo package -p nvidia-nrd --list
cargo package -p nvidia-omm --list
cargo package -p nvidia-streamline --list
```

- [ ] Run real resolver package verification and publish dry-run first:

```sh
unset DOCS_RS
cargo package -p nvidia-sdk
cargo publish -p nvidia-sdk --dry-run
```

- [ ] Confirm that the separately approved `nvidia-sdk` publication is visible in
  the crates.io index before attempting dependent verification.
- [ ] On provisioned native hosts with `DOCS_RS` unset, run in this order:

```sh
cargo package -p nvidia-dlss
cargo publish -p nvidia-dlss --dry-run
cargo package -p nvidia-nrd
cargo publish -p nvidia-nrd --dry-run
cargo package -p nvidia-omm --features native
cargo publish -p nvidia-omm --features native --dry-run
cargo package -p nvidia-streamline
cargo publish -p nvidia-streamline --dry-run
```

Use Linux for native DLSS and Windows for native Streamline verification. Other
hosts may only verify their stubs.

Package verification is compilation, not a runtime smoke test. Dry-runs do not
upload crates and do not prove final publication will succeed.

- [ ] Record the host, features, versions, and result for every command.
- [ ] Require explicit user approval again for wrapper publication and any
  subsequent tag/push actions. Preserve resolver-first ordering.
