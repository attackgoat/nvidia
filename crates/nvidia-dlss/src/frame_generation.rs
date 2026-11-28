use {crate::VulkanImage, ash::vk};

/// Fixed dimensions and backbuffer format for 2× SDR frame generation with UI
/// recomposition. A change requires creating a new feature after releasing the
/// old one.
#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(C)]
pub struct DlssFgConfig {
    pub output_extent: [u32; 2],
    pub guide_extent: [u32; 2],
    pub color_format: vk::Format,
}

/// Borrowed resources for one Frame Generation evaluation. Color images cover
/// the same viewport as depth and motion. The real and HUD-less color must
/// agree outside the UI.
///
/// All inputs require `SAMPLED` usage and `SHADER_READ_ONLY_OPTIMAL` layout.
/// The generated image requires `STORAGE` usage and `GENERAL` layout and may
/// not alias an input. The four color images must match the configured output
/// extent and format. Depth (`R32_SFLOAT`) and motion (`R16G16_SFLOAT`) match
/// the configured guide extent. All views have the color aspect, including
/// the shader-written hardware depth.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct DlssFgResources {
    pub real_color: VulkanImage,
    pub hudless_color: VulkanImage,
    /// Premultiplied UI color and opacity.
    pub ui_color: VulkanImage,
    /// Reversed hardware depth (near = 1, far = 0).
    pub depth: VulkanImage,
    /// Undilated, unjittered vectors from current to previous, including camera
    /// motion, with positive X to the right and positive Y down.
    pub motion: VulkanImage,
    pub generated_color: VulkanImage,
    /// A writable `STORAGE_BUFFER | SHADER_DEVICE_ADDRESS` with device-address
    /// allocation and at least four bytes.
    /// After GPU completion, a nonzero first byte means NGX copied the real
    /// color instead of generating an intermediate frame.
    pub suppression: vk::Buffer,
}

#[cfg(any(test, nvidia_dlss_native))]
impl DlssFgConfig {
    pub(crate) fn validate(self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.output_extent.contains(&0) && !self.guide_extent.contains(&0),
            "zero dlss frame generation extent"
        );
        anyhow::ensure!(
            self.color_format != vk::Format::UNDEFINED,
            "undefined dlss frame generation color format"
        );

        Ok(())
    }
}

#[cfg(any(test, nvidia_dlss_native))]
impl DlssFgResources {
    pub(crate) fn validate(&self, config: DlssFgConfig) -> anyhow::Result<()> {
        let color = |name, image| (name, image, config.output_extent, config.color_format);
        let guide_extent = config.guide_extent;
        for (name, image, extent, format) in [
            color("real_color", self.real_color),
            color("hudless_color", self.hudless_color),
            color("ui_color", self.ui_color),
            color("generated_color", self.generated_color),
            ("depth", self.depth, guide_extent, vk::Format::R32_SFLOAT),
            ("motion", self.motion, guide_extent, vk::Format::R16G16_SFLOAT),
        ] {
            anyhow::ensure!(
                image.image() != vk::Image::null() && image.view() != vk::ImageView::null(),
                "null dlss frame generation {name} image or view"
            );
            anyhow::ensure!(
                image.subresource_range().aspect_mask == vk::ImageAspectFlags::COLOR,
                "dlss frame generation {name} requires the color aspect"
            );
            anyhow::ensure!(
                image.extent() == extent && image.format() == format,
                "dlss frame generation {name} requires extent {extent:?} and format {}",
                format.as_raw()
            );

            let generated = name == "generated_color";
            let (usage, layout) = if generated {
                (vk::ImageUsageFlags::STORAGE, vk::ImageLayout::GENERAL)
            } else {
                (
                    vk::ImageUsageFlags::SAMPLED,
                    vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                )
            };
            anyhow::ensure!(
                image.usage().contains(usage) && image.layout() == layout,
                "dlss frame generation {name} requires usage {:#x} and layout {}",
                usage.as_raw(),
                layout.as_raw()
            );
            anyhow::ensure!(
                generated || image.image() != self.generated_color.image(),
                "dlss frame generation {name} aliases generated_color"
            );
        }

        anyhow::ensure!(
            self.suppression != vk::Buffer::null(),
            "null dlss frame generation suppression buffer"
        );

        Ok(())
    }
}

#[cfg(not(nvidia_dlss_native))]
mod unavailable {
    #[cfg(feature = "vk-graph")]
    use vk_graph::driver::instance::Instance;
    use {
        super::{DlssFgConfig, DlssFgResources, vk},
        crate::{DlssRrFrameConstants, Evaluator, Ngx},
        std::ffi::CString,
    };

    const UNAVAILABLE: &str = "dlss frame generation requires the native ngx backend";

    /// An owned Frame Generation feature. It cannot be created on this build.
    #[must_use]
    pub struct DlssFgEvaluator {
        _private: (),
    }

    impl DlssFgEvaluator {
        /// # Safety
        /// The native backend requires valid resources and a recording command buffer.
        pub unsafe fn evaluate(
            &mut self,
            _command_buffer: vk::CommandBuffer,
            _frame: &DlssRrFrameConstants,
            _resources: &DlssFgResources,
        ) -> anyhow::Result<()> {
            anyhow::bail!(UNAVAILABLE)
        }

        /// # Safety
        /// The native backend requires completion of all commands using this feature.
        pub unsafe fn release(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
    }

    impl Evaluator {
        pub fn frame_generation_available(&self) -> anyhow::Result<bool> {
            Ok(false)
        }

        /// # Safety
        /// The native backend requires a valid recording command buffer and enabled
        /// Frame Generation requirements.
        pub unsafe fn create_frame_generation(
            &self,
            _command_buffer: vk::CommandBuffer,
            _config: DlssFgConfig,
        ) -> anyhow::Result<DlssFgEvaluator> {
            anyhow::bail!(UNAVAILABLE)
        }
    }

    impl Ngx {
        pub fn frame_generation_instance_extensions(&self) -> anyhow::Result<Vec<CString>> {
            anyhow::bail!(UNAVAILABLE)
        }

        /// # Safety
        /// The physical device must have been enumerated from the live instance.
        #[cfg(feature = "vk-graph")]
        pub unsafe fn frame_generation_device_extensions(
            &self,
            _instance: &Instance,
            _physical_device: vk::PhysicalDevice,
        ) -> anyhow::Result<Vec<CString>> {
            anyhow::bail!(UNAVAILABLE)
        }

        /// # Safety
        /// The physical device must have been enumerated from the live instance.
        pub unsafe fn frame_generation_device_extensions_raw(
            &self,
            _instance: vk::Instance,
            _physical_device: vk::PhysicalDevice,
        ) -> anyhow::Result<Vec<CString>> {
            anyhow::bail!(UNAVAILABLE)
        }
    }
}

#[cfg(not(nvidia_dlss_native))]
pub use unavailable::DlssFgEvaluator;

#[cfg(test)]
mod tests {
    use {
        super::*,
        ash::vk::Handle as _,
        std::mem::{offset_of, size_of},
    };

    #[test]
    fn fg_ffi_layout() {
        assert_eq!(size_of::<DlssFgConfig>(), 20);
        assert_eq!(offset_of!(DlssFgConfig, color_format), 16);
        assert_eq!(size_of::<DlssFgResources>(), 392);
        assert_eq!(offset_of!(DlssFgResources, suppression), 384);
    }

    #[test]
    fn fg_evaluation_requires_fixed_resources_and_suppression() {
        let config = DlssFgConfig {
            output_extent: [1280, 720],
            guide_extent: [640, 360],
            color_format: vk::Format::R8G8B8A8_UNORM,
        };
        let image = |handle, extent, format, generated| {
            let (layout, usage) = if generated {
                (vk::ImageLayout::GENERAL, vk::ImageUsageFlags::STORAGE)
            } else {
                (
                    vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                    vk::ImageUsageFlags::SAMPLED,
                )
            };
            VulkanImage::from_raw(
                vk::Image::from_raw(handle),
                vk::ImageView::from_raw(handle),
                layout,
                extent,
                format,
                vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .level_count(1)
                    .layer_count(1),
                vk::ImageCreateFlags::empty(),
                usage,
            )
        };
        let color =
            |handle, generated| image(handle, config.output_extent, config.color_format, generated);
        let resources = DlssFgResources {
            real_color: color(1, false),
            hudless_color: color(2, false),
            ui_color: color(3, false),
            depth: image(4, config.guide_extent, vk::Format::R32_SFLOAT, false),
            motion: image(5, config.guide_extent, vk::Format::R16G16_SFLOAT, false),
            generated_color: color(6, true),
            suppression: vk::Buffer::from_raw(7),
        };

        assert!(config.validate().is_ok());
        assert!(resources.validate(config).is_ok());
        assert!(
            DlssFgConfig {
                guide_extent: [0, 360],
                ..config
            }
            .validate()
            .is_err()
        );
        assert!(
            DlssFgConfig {
                color_format: vk::Format::UNDEFINED,
                ..config
            }
            .validate()
            .is_err()
        );
        for changed in [
            DlssFgResources {
                motion: resources.depth,
                ..resources
            },
            DlssFgResources {
                depth: image(4, config.output_extent, vk::Format::R32_SFLOAT, false),
                ..resources
            },
            DlssFgResources {
                hudless_color: color(2, true),
                ..resources
            },
            DlssFgResources {
                generated_color: color(6, false),
                ..resources
            },
            DlssFgResources {
                generated_color: color(1, true),
                ..resources
            },
            DlssFgResources {
                suppression: vk::Buffer::null(),
                ..resources
            },
        ] {
            assert!(changed.validate(config).is_err());
        }
    }
}
