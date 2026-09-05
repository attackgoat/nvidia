# Contributing

Contributions are welcome through issues and pull requests.

## Checkout Requirements

> [!IMPORTANT]
> Checkouts must support real symbolic links. Git's plain-text symlink emulation
> is not a supported checkout configuration.

Crate-local licenses are relative symlinks to the root `LICENSE-MIT` and
`LICENSE-APACHE`. Edit only the root texts. Cargo packages their resolved
contents as regular files.

<details>
<summary>Windows checkout setup</summary>

Enable Developer Mode or obtain the required symlink privilege. Then clone with
symlink support enabled:

```console
git -c core.symlinks=true clone git@github.com:attackgoat/nvidia.git
```

</details>

## Additional Backends

Additional renderer adapters and graphics-API backends are welcome.

Keep renderer dependencies behind optional features. Preserve the default
`vk-graph` integration, and keep the core usable without it.

Include compilation coverage for both default features and graph-free builds.
Document the backend's capabilities and native SDK requirements. Explain its
resource ownership and shutdown safety contracts.

## Validation

Before submitting a change, run:

```console
cargo fmt --all --check
cargo clippy --all-targets --workspace -- -D warnings
cargo test --all-targets --workspace
```

Every crate inherits the workspace Clippy `all` and `pedantic` groups.

| Code | Unsafe-Code Policy |
| --- | --- |
| All crates | Deny implicit unsafe operations inside unsafe functions. |
| Native bindings | Use explicit unsafe FFI blocks and document their safety contracts. |
| SDK resolver, CLI, and its integration tests | Forbid unsafe code at the crate root. |

## Rust Style

- Keep imports grouped.
- Keep type definitions adjacent to their implementations.
- Alphabetize type groups and constants. Preserve native ABI field ordering.
- Borrow inputs that are not retained. Keep native handles typed.
- Give recoverable errors concise operation and path context.
- Separate conditional, logging, and unsafe phases with blank lines.
- Use lowercase diagnostic fragments.

## SDK Provenance

> [!WARNING]
> Do not commit downloaded SDK files, generated native libraries, credentials,
> or license-restricted NVIDIA content.

Changes to `sdk-lock.toml` must use official upstream URLs and cryptographic
hashes. Explain the provenance update in the pull request.

## Contribution License

Unless explicitly stated otherwise, contributions are licensed under
`MIT OR Apache-2.0` without additional terms.
