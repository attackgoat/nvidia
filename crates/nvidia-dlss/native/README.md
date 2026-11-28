# Native Regression Test

With `DLSS_SDK` pointing at the resolved DLSS 310.9.16 SDK, run from the repository root:

```sh
c++ -std=c++17 -I"$DLSS_SDK/include" crates/nvidia-dlss/native/ngx_test.cpp \
    "$DLSS_SDK/lib/Linux_x86_64/libnvsdk_ngx.a" -ldl -o /tmp/ngx_test
/tmp/ngx_test
```

The test runs the pinned SDK creation and evaluation helpers and intercepts only
the final GPU entrypoints. An in-memory `NVSDK_NGX_Parameter` implementation stores
the actual parameters set by the SDK. Tests cover all 12 convention combinations,
both reflection guides, effective per-axis zero-to-unity motion scaling (including
negative zero), view ranges, usage/layout validation and dimension rejection before
feature creation or evaluation.
It also alternates two feature histories with distinct parameter ownership,
changes the secondary extent without replacing the primary feature, and checks
independent release, repeated release and invalid view indices. These are mocked
GPU-entrypoint checks, not evidence of live SDK coexistence or GPU memory usage.
Compile without `NDEBUG` so assertions remain enabled.

## Configuration and Contracts

Pass `DlssRrConfig` to `Ngx::initialize` or `Ngx::initialize_raw`. It is copied into
the evaluator/context and cannot change during evaluation. Defaults retain HDR,
render-resolution primary motion, and reversed hardware depth. Forward hardware
and linear view-space depth are also supported; only reversed hardware sets
`DepthInverted`. Normal/roughness remains packed and the preset remains E.

Image dimensions and quality still come from each evaluation, with feature
recreation when they change. Complete prior GPU work first; this is not dynamic
resolution within a feature. Input color defines render resolution; output color
defines output resolution. Primary motion follows the configuration. Depth,
albedos, normal/roughness and reflection guides remain render-sized even with
output-sized primary motion. Output-sized motion must already be dilated.
This wrapper uses whole views, zero subrect offsets, and no output subrects.
All inputs need `SAMPLED` usage and `SHADER_READ_ONLY_OPTIMAL`; output needs
`STORAGE` and `GENERAL`. Validation checks declarations, not actual Vulkan objects.

Pinned DLSS 310.4.0 sources inspected:

- `doc/DLSS-RR Integration Guide.pdf`, sections 3.3, 3.4.1-3.4.9, 3.4.14 and 5.3:
  no dynamic resolution, render-sized guides, hardware or view-space depth, output sizing.
- `doc/DLSS_Programming_Guide_Release.pdf`, sections 3.4 and 3.6.2:
  sampled inputs/storage output and `MVLowRes` versus dilated output-resolution motion.
- `include/nvsdk_ngx_defs_dlssd.h`: `Depth_Type_Linear` and `Depth_Type_HW`.
- `include/nvsdk_ngx_helpers_dlssd_vk.h`: creation parameters and independent
  `InMVScaleX/Y == 0.0f ? 1.0f : InMVScaleX/Y` conversion before NGX evaluation.
- `include/nvsdk_ngx_vk.h`: read/write image resources require storage usage.

## Frame Generation (Linux x86-64)

Native builds compile the Frame Generation bindings and stage both DLSS
libraries from the selected development or release profile. Custom `DLSS_SDK`
directories must supply the DLSS-G headers and both profiles of the Ray
Reconstruction and Frame Generation libraries.

To stage both DLSS runtimes for packaging:

```sh
nvidia-sdk stage dlss --target x86_64-unknown-linux-gnu \
    --output <runtime-dir>
```

Both libraries use the same license and third-party notice.

Query `Ngx::frame_generation_instance_extensions` before creating the Vulkan
instance and `Ngx::frame_generation_device_extensions` (or the `_raw` variant)
before creating the selected device. Merge the results with RR requirements.
If FG discovery fails, use the RR extension queries. After NGX initialization,
query `Evaluator::frame_generation_available` before creating the FG feature.

The fixed-size 2× path uses SDR real color, HUD-less color, premultiplied UI,
reversed hardware depth, and undilated, unjittered motion. Use guide-pixel
motion with scale `[1.0, 1.0]`; the guide's positive axes are right and down.
Supply unjittered row-vector matrices and a clip-space jitter offset. All
inputs need `SAMPLED` usage and `SHADER_READ_ONLY_OPTIMAL`; the generated
image needs `STORAGE` and `GENERAL`. The SDK records graphics and compute
work; record creation and evaluation on a graphics-capable queue and
synchronize both. The four-byte suppression buffer needs `STORAGE_BUFFER |
SHADER_DEVICE_ADDRESS` and device-address memory. After GPU completion and
host-read synchronization, byte zero is zero for a generated midpoint and
nonzero for a real-frame copy.

After completion of submitted creation and evaluations, discard any
unsubmitted recordings, then explicitly release the evaluator. The native
feature has a separate parameter map and shares NGX initialization with RR.

The 310.9.16 release runtime on an RTX 5080 with driver 620.70.06 produced
the expected X midpoint with `[1.0, 1.0]` scaling, composited semitransparent
UI over the moving object, and suppressed interpolation on both initial and
subsequent reset. It released the feature after the last fence and after
resetting a never-submitted creation recording. The RR and FG features
coexisted on the same device, and RR's optimal extent query still worked
after FG release.
Vulkan validation reported layout errors on SDK-owned images during FG
draws (`VUID-vkCmdDraw-None-09600`); the same errors were present with the
application inputs in either `SHADER_READ_ONLY_OPTIMAL` or `GENERAL`.
