//! Vulkan DLSS Ray Reconstruction bindings through NGX.
//!
//! Use [`Ngx::native_backend_available`] to check whether the native backend was built
//! and [`Ngx::staged_runtime_dir`] to locate its staged runtime before creating a context.

use {
    ash::vk::{self, Handle as _},
    std::path::PathBuf,
};

pub const DLSS_RUNTIME_FILE: &str = "libnvidia-ngx-dlssd.so.310.4.0";
pub const SDK_VERSION: &str = "310.4.0";

/// Color encoding fixed for the lifetime of an initialized evaluator.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u32)]
pub enum DlssRrColor {
    /// Linear HDR color (before tone mapping).
    #[default]
    Hdr = 0,
    /// LDR color.
    Ldr = 1,
}

/// Conventions copied at initialization; changing them requires shutdown and reinitialization.
/// Dimensions and quality may still change between completed evaluations, recreating the feature.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(C)]
pub struct DlssRrConfig {
    pub color: DlssRrColor,
    pub motion_resolution: DlssRrMotionResolution,
    pub depth: DlssRrDepth,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u32)]
pub enum DlssRrDepth {
    /// Hardware depth: near = 1, far = 0.
    #[default]
    ReversedHardware = 0,
    /// Hardware depth: near = 0, far = 1.
    ForwardHardware = 1,
    /// Linear view-space depth, not normalized hardware depth.
    LinearViewSpace = 2,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct DlssRrFrameConstants {
    pub camera_view_to_clip: [f32; 16],
    pub clip_to_camera_view: [f32; 16],
    pub clip_to_prev_clip: [f32; 16],
    pub prev_clip_to_clip: [f32; 16],
    pub world_to_camera_view: [f32; 16],
    pub camera_view_to_world: [f32; 16],
    pub jitter_offset: [f32; 2],
    /// Per-axis multiplier converting stored motion vectors to render-pixel displacement.
    /// Use `[1.0, 1.0]` for pixel-space vectors, or `[render_width, render_height]`
    /// for UV-space vectors (with signs adjusted to the input convention).
    /// NGX converts each zero axis (including negative zero) to an effective `1.0`.
    /// Thus the derived default `[0.0, 0.0]` has effective unity scaling.
    pub mvec_scale: [f32; 2],
    pub camera_pos: [f32; 3],
    pub camera_near: f32,
    pub camera_up: [f32; 3],
    pub camera_far: f32,
    pub camera_right: [f32; 3],
    pub camera_fov: f32,
    pub camera_forward: [f32; 3],
    pub camera_aspect_ratio: f32,
    pub frame_index: u32,
    pub reset: u32,
}

/// Resolution of the primary motion field, not the reflection guide.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u32)]
pub enum DlssRrMotionResolution {
    #[default]
    Render = 0,
    /// Output-sized, already dilated motion vectors (NGX `MVLowRes` disabled).
    Output = 1,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u32)]
pub enum DlssRrQuality {
    #[default]
    Balanced = 0,
    Quality = 1,
    /// Native-resolution anti-aliasing; render and output dimensions must be equal.
    Dlaa = 2,
    Performance = 3,
    UltraPerformance = 4,
}

/// Images for NGX evaluation. All inputs, including depth and the reflection guide,
/// must be in `SHADER_READ_ONLY_OPTIMAL`; the output must be in `GENERAL`.
/// Evaluation validates the declared layouts but does not record layout transitions.
/// All inputs require `SAMPLED` usage; output requires `STORAGE`. Depth, albedos,
/// packed normal/roughness and either reflection guide must match input color dimensions.
/// Primary motion must match the configured render or output resolution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DlssRrResources {
    pub input_color: VulkanImage,
    pub output_color: VulkanImage,
    pub depth: VulkanImage,
    pub motion: VulkanImage,
    pub diffuse_albedo: VulkanImage,
    pub specular_albedo: VulkanImage,
    pub normal_roughness: VulkanImage,
    pub reflection_guide: ReflectionGuide,
}

#[cfg(any(nvidia_dlss_native, test))]
impl DlssRrResources {
    fn native(&self, config: DlssRrConfig) -> anyhow::Result<NativeDlssRrResources> {
        for (name, image) in [
            ("input_color", self.input_color),
            ("depth", self.depth),
            ("motion", self.motion),
            ("diffuse_albedo", self.diffuse_albedo),
            ("specular_albedo", self.specular_albedo),
            ("normal_roughness", self.normal_roughness),
            ("reflection_guide", self.reflection_guide.image()),
        ] {
            anyhow::ensure!(
                image.usage().contains(vk::ImageUsageFlags::SAMPLED),
                "ngx {name} requires vk_image_usage_sampled_bit"
            );

            let extent =
                if name == "motion" && config.motion_resolution == DlssRrMotionResolution::Output {
                    self.output_color.extent()
                } else {
                    self.input_color.extent()
                };

            anyhow::ensure!(
                image.extent() == extent && !extent.contains(&0),
                "ngx {name} dimensions must match the configured resolution {extent:?}"
            );
            anyhow::ensure!(
                image.layout() == vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                "ngx {name} must be in vk_image_layout_shader_read_only_optimal"
            );
        }

        anyhow::ensure!(
            self.output_color
                .usage()
                .contains(vk::ImageUsageFlags::STORAGE),
            "ngx output_color requires vk_image_usage_storage_bit"
        );
        anyhow::ensure!(
            !self.output_color.extent().contains(&0),
            "zero output extent"
        );

        anyhow::ensure!(
            self.output_color.layout() == vk::ImageLayout::GENERAL,
            "ngx output_color must be in vk_image_layout_general"
        );

        let (reflection_guide, reflection_guide_kind) = match self.reflection_guide {
            ReflectionGuide::SpecularMotion(image) => (image, 0),
            ReflectionGuide::SpecularHitDistance(image) => {
                anyhow::ensure!(
                    image.format() == vk::Format::R32_SFLOAT,
                    "specular hit distance must use vk_format_r32_sfloat"
                );

                (image, 1)
            }
        };

        Ok(NativeDlssRrResources {
            input_color: self.input_color,
            output_color: self.output_color,
            depth: self.depth,
            motion: self.motion,
            diffuse_albedo: self.diffuse_albedo,
            specular_albedo: self.specular_albedo,
            normal_roughness: self.normal_roughness,
            reflection_guide,
            reflection_guide_kind,
            reserved: 0,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Identity {
    kind: IdentityKind,
}

impl Identity {
    /// # Errors
    /// Returns an error if the application ID is zero.
    pub fn application_id(application_id: u64) -> anyhow::Result<Self> {
        anyhow::ensure!(
            application_id != 0,
            "nvidia application id must not be zero"
        );

        Ok(Self {
            kind: IdentityKind::ApplicationId(application_id),
        })
    }

    /// # Errors
    /// Returns an error if either string is empty or contains a null byte.
    pub fn project(
        project_id: impl Into<String>,
        engine_version: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let project_id = project_id.into();
        let engine_version = engine_version.into();

        anyhow::ensure!(
            !project_id.is_empty(),
            "nvidia project id must not be empty"
        );
        anyhow::ensure!(
            !engine_version.is_empty(),
            "nvidia engine version must not be empty"
        );
        anyhow::ensure!(
            !project_id.as_bytes().contains(&0),
            "nvidia project id contains a null byte"
        );
        anyhow::ensure!(
            !engine_version.as_bytes().contains(&0),
            "nvidia engine version contains a null byte"
        );

        Ok(Self {
            kind: IdentityKind::Project {
                project_id,
                engine_version,
            },
        })
    }

    #[must_use]
    pub fn application_id_value(&self) -> Option<u64> {
        match self.kind {
            IdentityKind::ApplicationId(value) => Some(value),
            IdentityKind::Project { .. } => None,
        }
    }

    #[must_use]
    pub fn project_id(&self) -> Option<&str> {
        match &self.kind {
            IdentityKind::ApplicationId(_) => None,
            IdentityKind::Project { project_id, .. } => Some(project_id),
        }
    }

    #[must_use]
    pub fn engine_version(&self) -> Option<&str> {
        match &self.kind {
            IdentityKind::ApplicationId(_) => None,
            IdentityKind::Project { engine_version, .. } => Some(engine_version),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum IdentityKind {
    ApplicationId(u64),
    Project {
        project_id: String,
        engine_version: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitInfo {
    pub identity: Identity,
    pub application_data_path: PathBuf,
    pub runtime_path: PathBuf,
}

impl InitInfo {
    #[must_use]
    pub fn new(
        identity: Identity,
        application_data_path: impl Into<PathBuf>,
        runtime_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            identity,
            application_data_path: application_data_path.into(),
            runtime_path: runtime_path.into(),
        }
    }
}

#[cfg(any(nvidia_dlss_native, test))]
#[derive(Clone, Copy)]
#[repr(C)]
struct NativeDlssRrResources {
    input_color: VulkanImage,
    output_color: VulkanImage,
    depth: VulkanImage,
    motion: VulkanImage,
    diffuse_albedo: VulkanImage,
    specular_albedo: VulkanImage,
    normal_roughness: VulkanImage,
    reflection_guide: VulkanImage,
    reflection_guide_kind: u32,
    reserved: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReflectionGuide {
    SpecularMotion(VulkanImage),
    SpecularHitDistance(VulkanImage),
}

impl ReflectionGuide {
    #[must_use]
    pub fn image(self) -> VulkanImage {
        match self {
            Self::SpecularMotion(image) | Self::SpecularHitDistance(image) => image,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct VulkanImage {
    image: u64,
    view: u64,
    state: u32,
    width: u32,
    height: u32,
    format: u32,
    aspect_mask: u32,
    base_mip_level: u32,
    level_count: u32,
    base_array_layer: u32,
    layer_count: u32,
    flags: u32,
    usage: u32,
}

impl VulkanImage {
    /// Describes a borrowed image view. `subresource_range` must match the range used
    /// to create `view`; `extent` is its base mip's size in pixels and `format` is
    /// the view format. This does not validate handles or transition image layouts.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn from_raw(
        image: vk::Image,
        view: vk::ImageView,
        layout: vk::ImageLayout,
        extent: [u32; 2],
        format: vk::Format,
        subresource_range: vk::ImageSubresourceRange,
        flags: vk::ImageCreateFlags,
        usage: vk::ImageUsageFlags,
    ) -> Self {
        Self {
            image: image.as_raw(),
            view: view.as_raw(),
            state: layout.as_raw().cast_unsigned(),
            width: extent[0],
            height: extent[1],
            format: format.as_raw().cast_unsigned(),
            aspect_mask: subresource_range.aspect_mask.as_raw(),
            base_mip_level: subresource_range.base_mip_level,
            level_count: subresource_range.level_count,
            base_array_layer: subresource_range.base_array_layer,
            layer_count: subresource_range.layer_count,
            flags: flags.as_raw(),
            usage: usage.as_raw(),
        }
    }

    #[must_use]
    pub fn image(self) -> vk::Image {
        vk::Image::from_raw(self.image)
    }

    #[must_use]
    pub fn view(self) -> vk::ImageView {
        vk::ImageView::from_raw(self.view)
    }

    #[must_use]
    pub fn layout(self) -> vk::ImageLayout {
        vk::ImageLayout::from_raw(self.state.cast_signed())
    }

    #[must_use]
    pub fn extent(self) -> [u32; 2] {
        [self.width, self.height]
    }

    #[must_use]
    pub fn format(self) -> vk::Format {
        vk::Format::from_raw(self.format.cast_signed())
    }

    #[must_use]
    pub fn usage(self) -> vk::ImageUsageFlags {
        vk::ImageUsageFlags::from_raw(self.usage)
    }

    pub fn subresource_range(self) -> vk::ImageSubresourceRange {
        vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::from_raw(self.aspect_mask),
            base_mip_level: self.base_mip_level,
            level_count: self.level_count,
            base_array_layer: self.base_array_layer,
            layer_count: self.layer_count,
        }
    }

    /// Gets a cached graph image view and describes its actual subresource range.
    /// The graph image must outlive GPU evaluation; `layout` must describe its
    /// state at evaluation time. No layout transitions are recorded here.
    ///
    /// # Errors
    /// Returns an error if the graph cannot create the requested view.
    #[cfg(feature = "vk-graph")]
    pub fn from_graph(
        image: &vk_graph::driver::image::Image,
        view: vk_graph::driver::image::ImageViewInfo,
        layout: vk::ImageLayout,
    ) -> anyhow::Result<Self> {
        Ok(Self::from_raw(
            image.handle,
            image.view(view)?,
            layout,
            [
                image
                    .info
                    .width
                    .checked_shr(view.base_mip_level)
                    .unwrap_or(0)
                    .max(1),
                image
                    .info
                    .height
                    .checked_shr(view.base_mip_level)
                    .unwrap_or(0)
                    .max(1),
            ],
            view.format,
            vk::ImageSubresourceRange {
                aspect_mask: view.aspect_mask,
                base_mip_level: view.base_mip_level,
                level_count: view.mip_level_count,
                base_array_layer: view.base_array_layer,
                layer_count: view.array_layer_count,
            },
            image.info.flags,
            image.info.usage,
        ))
    }
}

/// Borrowed Vulkan handles and entrypoints used to initialize NGX without a graph device.
#[derive(Clone, Copy)]
pub struct VulkanInitInfo {
    pub instance: vk::Instance,
    pub physical_device: vk::PhysicalDevice,
    pub device: vk::Device,
    pub get_instance_proc_addr: vk::PFN_vkGetInstanceProcAddr,
    pub get_device_proc_addr: vk::PFN_vkGetDeviceProcAddr,
}

#[cfg(nvidia_dlss_native)]
mod native;
#[cfg(not(nvidia_dlss_native))]
mod stub;

#[cfg(nvidia_dlss_native)]
pub use native::{Evaluator, Ngx};
#[cfg(not(nvidia_dlss_native))]
pub use stub::{Evaluator, Ngx};

#[cfg(test)]
mod tests {
    use {
        super::*,
        std::mem::{align_of, offset_of, size_of},
    };

    #[test]
    fn backend_discovery() {
        const AVAILABLE: bool = Ngx::native_backend_available();
        let runtime_dir: Option<&'static std::path::Path> = Ngx::staged_runtime_dir();
        assert_eq!(AVAILABLE, cfg!(nvidia_dlss_native));
        assert_eq!(runtime_dir.is_some(), AVAILABLE);

        #[cfg(nvidia_dlss_native)]
        assert_eq!(
            runtime_dir,
            Some(std::path::Path::new(env!("NVIDIA_DLSS_RUNTIME_DIR")))
        );
    }

    #[test]
    fn vulkan_api_signatures() {
        let _: unsafe fn(
            &Ngx,
            vk::Instance,
            vk::PhysicalDevice,
        ) -> anyhow::Result<Vec<std::ffi::CString>> = Ngx::device_extensions_raw;
        let _: unsafe fn(&mut Ngx, VulkanInitInfo, DlssRrConfig) -> anyhow::Result<Evaluator> =
            Ngx::initialize_raw;
        let _: unsafe fn(
            &Evaluator,
            vk::CommandBuffer,
            &DlssRrFrameConstants,
            &DlssRrResources,
            DlssRrQuality,
        ) -> anyhow::Result<()> = Evaluator::evaluate;
        let _: unsafe fn(&Evaluator) -> anyhow::Result<()> = Evaluator::shutdown;
        let _: unsafe fn(&mut Ngx) -> anyhow::Result<()> = Ngx::shutdown;

        #[cfg(feature = "vk-graph")]
        {
            use vk_graph::driver::{device::Device, instance::Instance};

            let _: unsafe fn(
                &Ngx,
                &Instance,
                vk::PhysicalDevice,
            ) -> anyhow::Result<Vec<std::ffi::CString>> = Ngx::device_extensions;
            let _: unsafe fn(&mut Ngx, &Device, DlssRrConfig) -> anyhow::Result<Evaluator> =
                Ngx::initialize;
            let _: fn(
                &vk_graph::driver::image::Image,
                vk_graph::driver::image::ImageViewInfo,
                vk::ImageLayout,
            ) -> anyhow::Result<VulkanImage> = VulkanImage::from_graph;
        }
    }

    #[test]
    fn native_ffi_layouts_are_stable() {
        assert_eq!(size_of::<DlssRrConfig>(), 12);
        assert_eq!(align_of::<DlssRrConfig>(), 4);
        assert_eq!(offset_of!(DlssRrConfig, motion_resolution), 4);
        assert_eq!(offset_of!(DlssRrConfig, depth), 8);
        assert_eq!(DlssRrColor::Hdr as u32, 0);
        assert_eq!(DlssRrColor::Ldr as u32, 1);
        assert_eq!(DlssRrMotionResolution::Render as u32, 0);
        assert_eq!(DlssRrMotionResolution::Output as u32, 1);
        assert_eq!(DlssRrDepth::ReversedHardware as u32, 0);
        assert_eq!(DlssRrDepth::ForwardHardware as u32, 1);
        assert_eq!(DlssRrDepth::LinearViewSpace as u32, 2);
        assert_eq!(
            DlssRrConfig::default(),
            DlssRrConfig {
                color: DlssRrColor::Hdr,
                motion_resolution: DlssRrMotionResolution::Render,
                depth: DlssRrDepth::ReversedHardware,
            }
        );
        assert_eq!(size_of::<DlssRrQuality>(), 4);
        assert_eq!(DlssRrQuality::default(), DlssRrQuality::Balanced);
        assert_eq!(DlssRrQuality::Balanced as u32, 0);
        assert_eq!(DlssRrQuality::Quality as u32, 1);
        assert_eq!(DlssRrQuality::Dlaa as u32, 2);
        assert_eq!(DlssRrQuality::Performance as u32, 3);
        assert_eq!(DlssRrQuality::UltraPerformance as u32, 4);

        assert_eq!(size_of::<VulkanImage>(), 64);
        assert_eq!(align_of::<VulkanImage>(), 8);
        assert_eq!(offset_of!(VulkanImage, aspect_mask), 32);
        assert_eq!(offset_of!(VulkanImage, base_mip_level), 36);
        assert_eq!(offset_of!(VulkanImage, level_count), 40);
        assert_eq!(offset_of!(VulkanImage, base_array_layer), 44);
        assert_eq!(offset_of!(VulkanImage, layer_count), 48);
        assert_eq!(offset_of!(VulkanImage, usage), 56);
        assert_eq!(size_of::<NativeDlssRrResources>(), 520);
        assert_eq!(
            offset_of!(NativeDlssRrResources, reflection_guide_kind),
            512
        );
        assert_eq!(size_of::<DlssRrFrameConstants>(), 472);
        assert_eq!(offset_of!(DlssRrFrameConstants, mvec_scale), 392);
    }

    #[test]
    fn raw_image_preserves_view_subresources() {
        for aspect_mask in [
            vk::ImageAspectFlags::COLOR,
            vk::ImageAspectFlags::DEPTH,
            vk::ImageAspectFlags::DEPTH | vk::ImageAspectFlags::STENCIL,
        ] {
            for (level_count, layer_count) in [
                (2, 3),
                (vk::REMAINING_MIP_LEVELS, vk::REMAINING_ARRAY_LAYERS),
            ] {
                let image = VulkanImage::from_raw(
                    vk::Image::from_raw(11),
                    vk::ImageView::from_raw(12),
                    vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                    [64, 32],
                    if aspect_mask == vk::ImageAspectFlags::COLOR {
                        vk::Format::R32_SFLOAT
                    } else {
                        vk::Format::D32_SFLOAT_S8_UINT
                    },
                    vk::ImageSubresourceRange {
                        aspect_mask,
                        base_mip_level: 4,
                        level_count,
                        base_array_layer: 5,
                        layer_count,
                    },
                    vk::ImageCreateFlags::empty(),
                    vk::ImageUsageFlags::SAMPLED,
                );
                let range = image.subresource_range();
                assert_eq!(range.aspect_mask.as_raw(), aspect_mask.as_raw());
                assert_eq!(range.base_mip_level, 4);
                assert_eq!(range.level_count, level_count);
                assert_eq!(range.base_array_layer, 5);
                assert_eq!(range.layer_count, layer_count);
                assert_eq!(image.image(), vk::Image::from_raw(11));
                assert_eq!(image.view(), vk::ImageView::from_raw(12));
                assert_eq!(image.extent(), [64, 32]);
                assert_eq!(
                    image.format().as_raw(),
                    if aspect_mask == vk::ImageAspectFlags::COLOR {
                        vk::Format::R32_SFLOAT
                    } else {
                        vk::Format::D32_SFLOAT_S8_UINT
                    }
                    .as_raw()
                );
            }
        }
    }

    #[test]
    fn evaluation_requires_ngx_image_layouts() {
        let input = VulkanImage::from_raw(
            vk::Image::from_raw(1),
            vk::ImageView::from_raw(2),
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            [64, 64],
            vk::Format::R32_SFLOAT,
            vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .level_count(1)
                .layer_count(1),
            vk::ImageCreateFlags::empty(),
            vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::STORAGE,
        );
        for reflection_guide in [
            ReflectionGuide::SpecularMotion(input),
            ReflectionGuide::SpecularHitDistance(input),
        ] {
            let resources = DlssRrResources {
                input_color: input,
                output_color: VulkanImage {
                    state: vk::ImageLayout::GENERAL.as_raw().cast_unsigned(),
                    ..input
                },
                depth: input,
                motion: input,
                diffuse_albedo: input,
                specular_albedo: input,
                normal_roughness: input,
                reflection_guide,
            };
            assert!(resources.native(DlssRrConfig::default()).is_ok());
            for index in 0..8 {
                for layout in [
                    vk::ImageLayout::UNDEFINED,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    vk::ImageLayout::GENERAL,
                    vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                ] {
                    let mut changed = resources;
                    let guide = match &mut changed.reflection_guide {
                        ReflectionGuide::SpecularMotion(image)
                        | ReflectionGuide::SpecularHitDistance(image) => image,
                    };
                    let images = [
                        &mut changed.input_color,
                        &mut changed.output_color,
                        &mut changed.depth,
                        &mut changed.motion,
                        &mut changed.diffuse_albedo,
                        &mut changed.specular_albedo,
                        &mut changed.normal_roughness,
                        guide,
                    ];
                    images[index].state = layout.as_raw().cast_unsigned();
                    let required = if index == 1 {
                        vk::ImageLayout::GENERAL
                    } else {
                        vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL
                    };
                    assert_eq!(
                        changed.native(DlssRrConfig::default()).is_ok(),
                        layout == required,
                        "image {index}"
                    );
                }
            }
        }
    }

    #[test]
    fn evaluation_requires_usage_and_configured_dimensions() {
        let input = VulkanImage::from_raw(
            vk::Image::from_raw(1),
            vk::ImageView::from_raw(2),
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            [64, 32],
            vk::Format::R32_SFLOAT,
            vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .level_count(1)
                .layer_count(1),
            vk::ImageCreateFlags::empty(),
            vk::ImageUsageFlags::SAMPLED,
        );
        let output = VulkanImage {
            width: 128,
            height: 64,
            state: vk::ImageLayout::GENERAL.as_raw().cast_unsigned(),
            usage: vk::ImageUsageFlags::STORAGE.as_raw(),
            ..input
        };
        for motion_resolution in [
            DlssRrMotionResolution::Render,
            DlssRrMotionResolution::Output,
        ] {
            let config = DlssRrConfig {
                motion_resolution,
                ..DlssRrConfig::default()
            };
            for reflection_guide in [
                ReflectionGuide::SpecularMotion(input),
                ReflectionGuide::SpecularHitDistance(input),
            ] {
                let resources = DlssRrResources {
                    input_color: input,
                    output_color: output,
                    depth: input,
                    motion: if motion_resolution == DlssRrMotionResolution::Render {
                        input
                    } else {
                        VulkanImage {
                            width: 128,
                            height: 64,
                            ..input
                        }
                    },
                    diffuse_albedo: input,
                    specular_albedo: input,
                    normal_roughness: input,
                    reflection_guide,
                };
                assert!(resources.native(config).is_ok());
                for index in 0..8 {
                    for invalid in 0..5 {
                        let mut changed = resources;
                        let guide = match &mut changed.reflection_guide {
                            ReflectionGuide::SpecularMotion(image)
                            | ReflectionGuide::SpecularHitDistance(image) => image,
                        };
                        let images = [
                            &mut changed.input_color,
                            &mut changed.output_color,
                            &mut changed.depth,
                            &mut changed.motion,
                            &mut changed.diffuse_albedo,
                            &mut changed.specular_albedo,
                            &mut changed.normal_roughness,
                            guide,
                        ];
                        let image = &mut *images[index];
                        match invalid {
                            0 => image.usage = 0,
                            1 => {
                                image.usage = if index == 1 {
                                    vk::ImageUsageFlags::SAMPLED
                                } else {
                                    vk::ImageUsageFlags::STORAGE
                                }
                                .as_raw();
                            }
                            2 => image.width = 0,
                            3 if index != 1 => image.width += 1,
                            4 if index != 1 => image.height += 1,
                            _ => continue,
                        }
                        assert!(
                            changed.native(config).is_err(),
                            "image {index}, invalid {invalid}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn identity_requires_non_empty_values() {
        assert!(Identity::application_id(0).is_err());
        assert!(Identity::project("", "1.0").is_err());
        assert!(Identity::project("project", "").is_err());
        assert!(Identity::project("project", "1.0").is_ok());
    }
}
