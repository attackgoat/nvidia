# Native Regression Test

With `DLSS_SDK` pointing at the resolved DLSS 310.4.0 SDK, run from the repository root:

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
