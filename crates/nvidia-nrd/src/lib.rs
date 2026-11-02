#[cfg(nvidia_nrd_native)]
mod native;

#[cfg(not(nvidia_nrd_native))]
mod stub;

#[cfg(nvidia_nrd_native)]
pub use native::Nrd;

#[cfg(not(nvidia_nrd_native))]
pub use stub::Nrd;

use {ash::vk, std::ffi::CStr};

/// Number of evaluation resource slots retained by NRD.
///
/// Conservative retirement rule: number every [`Nrd::evaluate`] call on a runtime,
/// including errors, independently of [`Frame::frame_index`]. Before call N, retire
/// every call through N - `QUEUED_EVALUATIONS`: its GPU work must have completed,
/// and its command buffer must never be submitted again. This also bounds work
/// recorded but not yet submitted; execute and wait for it, or discard it, before
/// its retirement deadline.
///
/// Pre-FFI errors (invalid layouts or a shut-down runtime) do not consume slots.
/// Native failures after `NewFrame` can consume slots, and returned errors do not
/// distinguish all failures before and after advancement. Counting all calls may
/// retire work early, which is safe. Use call age to retire all older work, not
/// call count modulo this capacity to infer which native slot is being reused.
/// NRD does not wait for GPU completion when recycling slots.
pub const QUEUED_EVALUATIONS: u8 = 3;

#[cfg(nvidia_nrd_native)]
const REQUIRED_DEVICE_EXTENSIONS: &[&CStr] = &[c"VK_KHR_push_descriptor"];

pub const SDK_VERSION: &str = "4.17.3";

#[cfg(any(nvidia_nrd_native, test))]
const VULKAN_1_4: u32 = vk::make_api_version(0, 1, 4, 0);

#[cfg(any(nvidia_nrd_native, test))]
fn validate_rectangles(
    resource: [u32; 2],
    current: [u32; 2],
    previous: [u32; 2],
) -> anyhow::Result<()> {
    anyhow::ensure!(
        (0..2).all(|axis| resource[axis] > 0
            && resource[axis] <= u32::from(u16::MAX)
            && current[axis] > 0
            && current[axis] <= resource[axis]
            && previous[axis] > 0
            && previous[axis] <= resource[axis]),
        "nrd active rectangles must fit nonzero u16 resource dimensions"
    );
    Ok(())
}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct Frame {
    pub view_to_clip: [f32; 16],
    pub view_to_clip_previous: [f32; 16],
    pub world_to_view: [f32; 16],
    pub world_to_view_previous: [f32; 16],
    pub motion_scale: [f32; 3],
    pub jitter: [f32; 2],
    pub jitter_previous: [f32; 2],
    pub denoising_range: f32,
    pub width: u32,
    pub height: u32,
    /// Fixed allocation dimensions shared by every input and output image.
    pub resource_width: u32,
    pub resource_height: u32,
    /// Previous frame's active rectangle, for dynamic-resolution reprojection.
    pub previous_width: u32,
    pub previous_height: u32,
    pub frame_index: u32,
    pub reset: u32,
    pub settings: RelaxSettings,
}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct Image {
    pub image: u64,
    pub format: i32,
    /// Initial and restored layout. Inputs must use `SHADER_READ_ONLY_OPTIMAL`;
    /// outputs must use `GENERAL`. Other declarations are rejected by `evaluate`.
    pub layout: i32,
}

impl Image {
    #[must_use]
    pub fn new(image: vk::Image, format: vk::Format, layout: vk::ImageLayout) -> Self {
        use vk::Handle as _;

        Self {
            image: image.as_raw(),
            format: format.as_raw(),
            layout: layout.as_raw(),
        }
    }
}

impl Nrd {
    /// Vulkan device extensions that must be enabled before creating the NRD runtime.
    ///
    /// NRD requires standard Vulkan 1.3 or later; this list does not imply support for
    /// earlier API versions. Enable `synchronization2`, `dynamicRendering`,
    /// `maintenance4`, `timelineSemaphore`, and `descriptorBindingPartiallyBound` on
    /// the logical device. Also enable `bufferDeviceAddress` when physically supported:
    /// NRI infers its use from physical-device support, not the enabled feature chain.
    /// For Vulkan 1.4 or later, enable the core `pushDescriptor`, `maintenance5`, and
    /// `maintenance6` features; the push-descriptor extension is no longer needed.
    /// Raw handles cannot be used to verify enabled device features.
    #[must_use]
    pub fn required_device_extensions(api_version: u32) -> &'static [&'static CStr] {
        #[cfg(nvidia_nrd_native)]
        if api_version < VULKAN_1_4 {
            return REQUIRED_DEVICE_EXTENSIONS;
        }

        #[cfg(not(nvidia_nrd_native))]
        let _ = api_version;

        &[]
    }

    #[cfg(any(nvidia_nrd_native, test))]
    fn validate_api_version(api_version: u32) -> anyhow::Result<()> {
        anyhow::ensure!(
            vk::api_version_variant(api_version) == 0
                && vk::api_version_major(api_version) == 1
                && (3..=u32::from(u8::MAX)).contains(&vk::api_version_minor(api_version)),
            "nrd requires standard vulkan 1.3 or later with a minor version representable by nri"
        );

        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct RelaxSettings {
    pub diffuse_max_accumulated_frame_num: u32,
    pub diffuse_max_fast_accumulated_frame_num: u32,
    pub history_fix_frame_num: u32,
    pub atrous_iteration_num: u32,
    pub diffuse_prepass_blur_radius: f32,
    pub min_hit_distance_weight: f32,
    pub diffuse_phi_luminance: f32,
    pub depth_threshold: f32,
    pub hit_distance_reconstruction_mode: u32,
    pub enable_anti_firefly: u32,
}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct Resources {
    pub motion: Image,
    pub normal_roughness: Image,
    pub view_z: Image,
    pub diffuse_sh0: Image,
    pub diffuse_sh1: Image,
    pub specular_sh0: Image,
    pub specular_sh1: Image,
    pub output_diffuse_sh0: Image,
    pub output_diffuse_sh1: Image,
    pub output_specular_sh0: Image,
    pub output_specular_sh1: Image,
}

#[cfg(any(nvidia_nrd_native, test))]
impl Resources {
    fn validate_layouts(&self) -> anyhow::Result<()> {
        for image in [
            self.motion,
            self.normal_roughness,
            self.view_z,
            self.diffuse_sh0,
            self.diffuse_sh1,
            self.specular_sh0,
            self.specular_sh1,
        ] {
            anyhow::ensure!(
                image.layout == vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL.as_raw(),
                "nrd input images must be in shader_read_only_optimal"
            );
        }

        for image in [
            self.output_diffuse_sh0,
            self.output_diffuse_sh1,
            self.output_specular_sh0,
            self.output_specular_sh1,
        ] {
            anyhow::ensure!(
                image.layout == vk::ImageLayout::GENERAL.as_raw(),
                "nrd output images must be in general"
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dynamic_rectangles_must_fit_the_fixed_allocation() {
        for (current, previous) in [
            ([960, 540], [427, 240]),
            ([427, 240], [960, 540]),
            ([693, 390], [691, 389]),
        ] {
            assert!(validate_rectangles([960, 540], current, previous).is_ok());
        }
        for (resource, current, previous) in [
            ([960, 540], [0, 240], [960, 540]),
            ([960, 540], [961, 540], [960, 540]),
            ([960, 540], [427, 240], [960, 541]),
            ([0, 540], [1, 240], [1, 240]),
            ([65536, 540], [427, 240], [427, 240]),
        ] {
            assert!(validate_rectangles(resource, current, previous).is_err());
        }
    }

    #[test]
    fn rejects_unsupported_api_versions_without_ffi() {
        for version in [
            vk::API_VERSION_1_0,
            vk::API_VERSION_1_1,
            vk::API_VERSION_1_2,
            vk::make_api_version(1, 1, 3, 0),
            vk::make_api_version(0, 2, 0, 0),
            vk::make_api_version(0, 1, 256, 0),
        ] {
            assert!(Nrd::validate_api_version(version).is_err());
        }

        for version in [vk::API_VERSION_1_3, VULKAN_1_4] {
            assert!(Nrd::validate_api_version(version).is_ok());
        }
    }

    #[test]
    fn checks_every_input_and_output_layout() {
        let input = Image::new(
            vk::Image::null(),
            vk::Format::R16G16B16A16_SFLOAT,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        );
        let output = Image {
            layout: vk::ImageLayout::GENERAL.as_raw(),
            ..input
        };
        let valid = Resources {
            motion: input,
            normal_roughness: input,
            view_z: input,
            diffuse_sh0: input,
            diffuse_sh1: input,
            specular_sh0: input,
            specular_sh1: input,
            output_diffuse_sh0: output,
            output_diffuse_sh1: output,
            output_specular_sh0: output,
            output_specular_sh1: output,
        };
        assert!(valid.validate_layouts().is_ok());

        for slot in 0..11 {
            for layout in [
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                if slot < 7 {
                    vk::ImageLayout::GENERAL
                } else {
                    vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL
                },
            ] {
                let mut resources = valid;
                let images = [
                    &mut resources.motion,
                    &mut resources.normal_roughness,
                    &mut resources.view_z,
                    &mut resources.diffuse_sh0,
                    &mut resources.diffuse_sh1,
                    &mut resources.specular_sh0,
                    &mut resources.specular_sh1,
                    &mut resources.output_diffuse_sh0,
                    &mut resources.output_diffuse_sh1,
                    &mut resources.output_specular_sh0,
                    &mut resources.output_specular_sh1,
                ];
                images[slot].layout = layout.as_raw();
                assert!(resources.validate_layouts().is_err(), "slot {slot}");
            }
        }
    }

    #[test]
    fn image_preserves_vulkan_handles_and_metadata() {
        use vk::Handle as _;

        let image = Image::new(
            vk::Image::from_raw(0x1234),
            vk::Format::R16G16B16A16_SFLOAT,
            vk::ImageLayout::GENERAL,
        );
        assert_eq!(image.image, 0x1234);
        assert_eq!(image.format, vk::Format::R16G16B16A16_SFLOAT.as_raw());
        assert_eq!(image.layout, vk::ImageLayout::GENERAL.as_raw());
    }

    #[test]
    fn ffi_layout_matches_native_bridge() {
        assert_eq!(size_of::<Image>(), 16);
        assert_eq!(size_of::<Resources>(), 176);
        assert_eq!(size_of::<RelaxSettings>(), 40);
        assert_eq!(size_of::<Frame>(), 360);
    }

    #[test]
    fn pre_vulkan_1_4_requires_push_descriptors() {
        #[cfg(nvidia_nrd_native)]
        assert_eq!(
            Nrd::required_device_extensions(vk::API_VERSION_1_3),
            [c"VK_KHR_push_descriptor"]
        );

        assert!(Nrd::required_device_extensions(VULKAN_1_4).is_empty());
    }
}
