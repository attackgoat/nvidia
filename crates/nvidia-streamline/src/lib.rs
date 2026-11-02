use {
    anyhow::{Result, ensure},
    ash::vk,
    std::{
        path::{Path, PathBuf},
        sync::Arc,
    },
};

#[cfg(all(target_os = "windows", feature = "vk-graph"))]
use {
    ash::vk::Handle as _,
    vk_graph::driver::{image::Image, instance::Instance},
};

#[cfg(nvidia_streamline_native)]
mod ffi;
#[cfg(nvidia_streamline_native)]
mod native;
#[cfg(not(nvidia_streamline_native))]
mod stub;

#[cfg(nvidia_streamline_native)]
use native as backend;
#[cfg(not(nvidia_streamline_native))]
use stub as backend;

/// Whether this build includes the native Streamline backend.
pub const NATIVE_BACKEND_AVAILABLE: bool = cfg!(nvidia_streamline_native);

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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u32)]
pub enum DlssRrQuality {
    #[default]
    Balanced = 0,
    Quality = 1,
    Dlaa = 2,
    Performance = 3,
    UltraPerformance = 4,
}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
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

#[derive(Clone)]
pub struct Evaluator {
    context: Arc<backend::Context>,
}

impl Evaluator {
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.context.is_active()
    }

    /// # Safety
    ///
    /// Unless null (which returns false), `physical_device` must be a valid handle enumerated
    /// from this context's interposer instance. The physical device, its parent instance, and
    /// the Vulkan/interposer loaders and their function pointers must remain valid throughout
    /// the call; do not destroy the instance or unload either loader concurrently.
    #[must_use]
    pub unsafe fn is_dlss_rr_supported(&self, physical_device: vk::PhysicalDevice) -> bool {
        unsafe { self.context.is_dlss_rr_supported(physical_device) }
    }

    /// # Errors
    ///
    /// Returns an error if the backend is unavailable, the context is shut down,
    /// the output extent is zero, or Streamline fails to return a nonzero render extent.
    pub fn optimal_render_extent(
        &self,
        output_extent: [u32; 2],
        quality: DlssRrQuality,
    ) -> Result<[u32; 2]> {
        self.context.optimal_render_extent(output_extent, quality)
    }

    /// # Safety
    ///
    /// This evaluator and its owning Streamline context must remain valid until execution
    /// completes. The command buffer must be recording on the context's device; every image and
    /// view must belong to that device, and each declared layout must match its actual state. All
    /// handles must remain valid until GPU execution completes. If dimensions or quality change,
    /// prior evaluations must have completed first.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend is unavailable, the context is shut down,
    /// the command buffer is null, or Streamline rejects the evaluation.
    pub unsafe fn evaluate_dlss_rr(
        &self,
        command_buffer: vk::CommandBuffer,
        frame: &DlssRrFrameConstants,
        resources: &DlssRrResources,
        quality: DlssRrQuality,
    ) -> Result<()> {
        unsafe {
            self.context
                .evaluate_dlss_rr(command_buffer, frame, resources, quality)
        }
    }

    /// # Safety
    ///
    /// All Streamline GPU work must have completed before shutdown.
    ///
    /// # Errors
    ///
    /// Returns an error if native Streamline shutdown fails. An already shut-down
    /// context or unavailable backend is a successful no-op.
    pub unsafe fn shutdown(&self) -> Result<()> {
        unsafe { self.context.shutdown() }
    }
}

/// Owns the Vulkan instance created by the Streamline interposer.
///
/// Shut Streamline down after GPU completion but before destroying child devices, then drop the
/// imported instance wrappers and all children before dropping this owner. The owner retains the
/// interposer loader and ash instance, independently of the `vk-graph` feature. If Streamline is
/// still active or the thread is panicking, dropping the owner intentionally leaks the native proxy
/// and context. Child lifetimes are the caller's responsibility; they are not tracked by this owner.
#[cfg(target_os = "windows")]
pub struct ProxyInstanceOwner {
    owner: Option<backend::ProxyOwner>,
    evaluator: Option<Evaluator>,
}

#[cfg(target_os = "windows")]
impl ProxyInstanceOwner {
    /// # Safety
    ///
    /// This owner must outlive all uses and clones of the returned loader and its function pointers.
    /// Do not create additional instances through this entry; use the owned interposer instance.
    ///
    /// # Panics
    ///
    /// Panics if the internal owner invariant is violated.
    #[must_use]
    pub unsafe fn entry(&self) -> &ash::Entry {
        self.owner
            .as_ref()
            .expect("proxy vulkan instance owner invariant")
            .entry()
    }

    /// # Panics
    ///
    /// Panics if the internal owner invariant is violated.
    #[must_use]
    pub fn evaluator(&self) -> Evaluator {
        self.evaluator
            .as_ref()
            .expect("proxy vulkan instance owner invariant")
            .clone()
    }

    /// # Safety
    ///
    /// Do not destroy this instance yourself. This owner must outlive all instance clones, function
    /// pointers, and child objects. After GPU completion, shut the evaluator down before destroying
    /// child devices, then destroy all children before dropping this owner.
    ///
    /// # Panics
    ///
    /// Panics if the internal owner invariant is violated.
    #[must_use]
    pub unsafe fn instance(&self) -> &ash::Instance {
        self.owner
            .as_ref()
            .expect("proxy vulkan instance owner invariant")
            .instance()
    }
}

#[cfg(target_os = "windows")]
impl Drop for ProxyInstanceOwner {
    fn drop(&mut self) {
        if std::thread::panicking() || self.evaluator.as_ref().is_some_and(Evaluator::is_active) {
            if let Some(owner) = self.owner.take() {
                std::mem::forget(owner);
            }

            if let Some(evaluator) = self.evaluator.take() {
                std::mem::forget(evaluator);
            }

            return;
        }

        if let Some(mut owner) = self.owner.take() {
            owner.destroy();
        }

        self.evaluator.take();
    }
}

/// A vk-graph instance created through the Streamline Vulkan interposer.
///
/// Keep this owner alive while using the instance. After GPU work completes, shut its evaluator
/// down before dropping child devices. The owner may be dropped only after every child and cloned
/// instance has been dropped. Dropping while Streamline is active or the thread is panicking
/// intentionally leaks the native proxy; child lifetimes are not tracked.
#[cfg(all(target_os = "windows", feature = "vk-graph"))]
pub struct ProxyVulkanInstance {
    instance: Option<Instance>,
    owner: Option<backend::ProxyOwner>,
    evaluator: Option<Evaluator>,
}

#[cfg(all(target_os = "windows", feature = "vk-graph"))]
impl ProxyVulkanInstance {
    /// # Safety
    ///
    /// After GPU completion, the evaluator must be shut down before child devices are destroyed.
    /// Every child object and cloned `vk_graph::Instance` created from the returned instance must
    /// then be destroyed before this owner.
    ///
    /// # Panics
    ///
    /// Panics if the internal owner invariant is violated.
    #[must_use]
    pub unsafe fn instance(&self) -> &Instance {
        self.instance
            .as_ref()
            .expect("proxy vulkan instance owner invariant")
    }

    /// # Panics
    ///
    /// Panics if the internal owner invariant is violated.
    #[must_use]
    pub fn evaluator(&self) -> Evaluator {
        self.evaluator
            .as_ref()
            .expect("proxy vulkan instance owner invariant")
            .clone()
    }

    /// # Safety
    ///
    /// The returned owner must outlive the instance and every device or resource created from it.
    /// After GPU completion, shut the evaluator down before destroying child devices.
    ///
    /// # Panics
    ///
    /// Panics if the internal owner invariant is violated.
    #[must_use]
    pub unsafe fn into_parts(mut self) -> (Instance, ProxyInstanceOwner) {
        let instance = self
            .instance
            .take()
            .expect("proxy vulkan instance owner invariant");
        let owner = ProxyInstanceOwner {
            owner: self.owner.take(),
            evaluator: self.evaluator.take(),
        };
        (instance, owner)
    }
}

#[cfg(all(target_os = "windows", feature = "vk-graph"))]
impl Drop for ProxyVulkanInstance {
    fn drop(&mut self) {
        if std::thread::panicking() || self.evaluator.as_ref().is_some_and(Evaluator::is_active) {
            if let Some(instance) = self.instance.take() {
                std::mem::forget(instance);
            }

            if let Some(owner) = self.owner.take() {
                std::mem::forget(owner);
            }

            if let Some(evaluator) = self.evaluator.take() {
                std::mem::forget(evaluator);
            }

            return;
        }

        drop(self.instance.take());

        if let Some(mut owner) = self.owner.take() {
            owner.destroy();
        }

        self.evaluator.take();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct ReflectionGuide {
    image: VulkanImage,
    kind: ReflectionGuideKind,
    reserved: u32,
}

impl ReflectionGuide {
    #[must_use]
    pub fn specular_motion_vectors(image: VulkanImage) -> Self {
        Self {
            image,
            kind: ReflectionGuideKind::SpecularMotionVectors,
            reserved: 0,
        }
    }

    /// # Errors
    ///
    /// Returns an error unless the image format is `VK_FORMAT_R32_SFLOAT`.
    pub fn specular_hit_distance(image: VulkanImage) -> Result<Self> {
        ensure!(
            image.format == vk::Format::R32_SFLOAT.as_raw().cast_unsigned(),
            "a specular hit-distance guide must use vk_format_r32_sfloat"
        );

        Ok(Self {
            image,
            kind: ReflectionGuideKind::SpecularHitDistance,
            reserved: 0,
        })
    }

    #[must_use]
    pub fn image(self) -> VulkanImage {
        self.image
    }

    #[must_use]
    pub fn kind(self) -> ReflectionGuideKind {
        self.kind
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum ReflectionGuideKind {
    SpecularMotionVectors = 0,
    SpecularHitDistance = 1,
}

pub struct Streamline {
    evaluator: Evaluator,
    runtime_dir: PathBuf,
    #[cfg(target_os = "windows")]
    interposer_path: PathBuf,
    #[cfg(target_os = "windows")]
    proxy_created: bool,
}

impl Streamline {
    /// Returns the build-staged Streamline runtime directory on the supported target.
    #[must_use]
    pub fn staged_runtime_dir() -> Option<&'static Path> {
        #[cfg(nvidia_streamline_native)]
        {
            Some(Path::new(env!("NVIDIA_STREAMLINE_RUNTIME_DIR")))
        }
        #[cfg(not(nvidia_streamline_native))]
        {
            None
        }
    }

    fn validate_project_identity(project_id: &str, engine_version: &str) -> Result<()> {
        ensure!(
            !project_id.is_empty(),
            "nvidia project id must not be empty"
        );
        ensure!(
            !engine_version.is_empty(),
            "nvidia engine version must not be empty"
        );
        ensure!(
            !project_id.as_bytes().contains(&0),
            "nvidia project id contains a null byte"
        );
        ensure!(
            !engine_version.as_bytes().contains(&0),
            "nvidia engine version contains a null byte"
        );

        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if either identity string is empty or contains a null byte,
    /// the native backend is unavailable, the runtime directory is not absolute or
    /// lacks the interposer, another context is active, or native initialization fails.
    pub fn initialize(
        project_id: &str,
        engine_version: &str,
        runtime_dir: impl AsRef<Path>,
    ) -> Result<Self> {
        Self::validate_project_identity(project_id, engine_version)?;

        let runtime_dir = runtime_dir.as_ref().to_owned();
        let context = Arc::new(backend::Context::initialize(
            project_id,
            engine_version,
            &runtime_dir,
        )?);
        #[cfg(target_os = "windows")]
        let interposer_path = runtime_dir.join("sl.interposer.dll");

        Ok(Self {
            evaluator: Evaluator { context },
            runtime_dir,
            #[cfg(target_os = "windows")]
            interposer_path,
            #[cfg(target_os = "windows")]
            proxy_created: false,
        })
    }

    #[must_use]
    pub fn runtime_dir(&self) -> &Path {
        &self.runtime_dir
    }

    #[must_use]
    pub fn evaluator(&self) -> Evaluator {
        self.evaluator.clone()
    }

    /// # Safety
    ///
    /// Unless null (which returns false), `physical_device` must be a valid handle enumerated
    /// from this context's interposer instance. The physical device, its parent instance, and
    /// the Vulkan/interposer loaders and their function pointers must remain valid throughout
    /// the call; do not destroy the instance or unload either loader concurrently.
    #[must_use]
    pub unsafe fn is_dlss_rr_supported(&self, physical_device: vk::PhysicalDevice) -> bool {
        unsafe { self.evaluator.is_dlss_rr_supported(physical_device) }
    }

    /// # Errors
    ///
    /// Returns the errors described by [`Evaluator::optimal_render_extent`].
    pub fn optimal_render_extent(
        &self,
        output_extent: [u32; 2],
        quality: DlssRrQuality,
    ) -> Result<[u32; 2]> {
        self.evaluator.optimal_render_extent(output_extent, quality)
    }

    /// # Safety
    ///
    /// This evaluator and its owning Streamline context must remain valid until execution
    /// completes. The command buffer must be recording on the context's device; every image and
    /// view must belong to that device, and each declared layout must match its actual state. All
    /// handles must remain valid until GPU execution completes. If dimensions or quality change,
    /// prior evaluations must have completed first.
    ///
    /// # Errors
    ///
    /// Returns the errors described by [`Evaluator::evaluate_dlss_rr`].
    pub unsafe fn evaluate_dlss_rr(
        &self,
        command_buffer: vk::CommandBuffer,
        frame: &DlssRrFrameConstants,
        resources: &DlssRrResources,
        quality: DlssRrQuality,
    ) -> Result<()> {
        unsafe {
            self.evaluator
                .evaluate_dlss_rr(command_buffer, frame, resources, quality)
        }
    }

    /// Creates an ash instance through the Streamline interposer without requiring `vk-graph`.
    ///
    /// Only one proxy may be successfully created per context, across both creation APIs.
    /// Devices must be created through this instance so the interposer observes their creation;
    /// this does not register externally created devices.
    ///
    /// # Safety
    ///
    /// `create_info` must satisfy Vulkan's instance creation requirements. Every reachable
    /// pointer (including application info, name arrays and strings, and the entire `p_next`
    /// chain) must be correctly aligned, initialized, and valid for its declared type and
    /// length throughout the call. Any callbacks and user data retained by Vulkan must remain
    /// valid for all possible invocations, including instance destruction.
    ///
    /// The returned owner must outlive all imported wrappers, instance/loader clones, function
    /// pointers, and child objects. Do not destroy its instance yourself. After GPU completion,
    /// shut the evaluator down before destroying child devices, then destroy all children and
    /// drop imported wrappers before dropping the owner and unloading its interposer.
    ///
    /// # Errors
    ///
    /// Returns an error if a proxy already exists, the backend is unavailable,
    /// the context is shut down, or loading the interposer or creating the instance fails.
    #[cfg(target_os = "windows")]
    pub unsafe fn create_proxy_ash_instance(
        &mut self,
        create_info: &vk::InstanceCreateInfo<'_>,
    ) -> Result<ProxyInstanceOwner> {
        ensure!(
            !self.proxy_created,
            "a streamline proxy vulkan instance has already been created"
        );

        let owner = unsafe {
            self.evaluator
                .context
                .create_proxy_ash_instance(&self.interposer_path, create_info)?
        };

        self.proxy_created = true;
        Ok(ProxyInstanceOwner {
            owner: Some(owner),
            evaluator: Some(self.evaluator.clone()),
        })
    }

    /// Creates a vk-graph instance through the Streamline interposer.
    ///
    /// Only one proxy may be successfully created per context, across both creation APIs.
    /// Devices must be created through this instance so the interposer observes their creation;
    /// this does not register externally created devices.
    ///
    /// # Safety
    ///
    /// `create_info` must satisfy Vulkan's instance creation requirements. Every reachable
    /// pointer (including application info, name arrays and strings, and the entire `p_next`
    /// chain) must be correctly aligned, initialized, and valid for its declared type and
    /// length throughout the call. Any callbacks and user data retained by Vulkan must remain
    /// valid for all possible invocations, including instance destruction.
    ///
    /// The returned owner must outlive all imported wrappers, instance/loader clones, function
    /// pointers, and child objects. Do not destroy its instance yourself. After GPU completion,
    /// shut the evaluator down before destroying child devices, then destroy all children and
    /// drop imported wrappers before dropping the owner and unloading its interposer.
    ///
    /// # Errors
    ///
    /// Returns an error if a proxy already exists, the backend is unavailable,
    /// the context is shut down, or loading, creating, or importing the instance fails.
    #[cfg(all(target_os = "windows", feature = "vk-graph"))]
    pub unsafe fn create_proxy_vulkan_instance(
        &mut self,
        create_info: &vk::InstanceCreateInfo<'_>,
    ) -> Result<ProxyVulkanInstance> {
        ensure!(
            !self.proxy_created,
            "a streamline proxy vulkan instance has already been created"
        );

        let (instance, owner) = unsafe {
            self.evaluator
                .context
                .create_proxy_vulkan_instance(&self.interposer_path, create_info)?
        };

        self.proxy_created = true;
        Ok(ProxyVulkanInstance {
            instance: Some(instance),
            owner: Some(owner),
            evaluator: Some(self.evaluator.clone()),
        })
    }

    /// # Safety
    ///
    /// All Streamline GPU work must have completed before shutdown.
    ///
    /// # Errors
    ///
    /// Returns the errors described by [`Evaluator::shutdown`].
    pub unsafe fn shutdown(&self) -> Result<()> {
        unsafe { self.evaluator.shutdown() }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(C)]
pub struct VulkanImage {
    pub image: u64,
    pub view: u64,
    pub state: u32,
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub mip_levels: u32,
    pub array_layers: u32,
    pub flags: u32,
    pub usage: u32,
}

impl VulkanImage {
    #[cfg(all(target_os = "windows", feature = "vk-graph"))]
    #[must_use]
    pub fn with_layout(image: &Image, view: vk::ImageView, layout: vk::ImageLayout) -> Self {
        Self {
            image: image.handle.as_raw(),
            view: view.as_raw(),
            state: layout.as_raw().cast_unsigned(),
            width: image.info.width,
            height: image.info.height,
            format: image.info.format.as_raw().cast_unsigned(),
            mip_levels: image.info.mip_level_count,
            array_layers: image.info.array_layer_count,
            flags: image.info.flags.as_raw(),
            usage: image.info.usage.as_raw(),
        }
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        std::mem::{align_of, offset_of, size_of},
    };

    #[test]
    fn native_ffi_layouts_are_stable() {
        assert_eq!(size_of::<DlssRrQuality>(), 4);
        assert_eq!(DlssRrQuality::default(), DlssRrQuality::Balanced);
        assert_eq!(DlssRrQuality::Balanced as u32, 0);
        assert_eq!(DlssRrQuality::Quality as u32, 1);
        assert_eq!(DlssRrQuality::Dlaa as u32, 2);
        assert_eq!(DlssRrQuality::Performance as u32, 3);
        assert_eq!(DlssRrQuality::UltraPerformance as u32, 4);

        assert_eq!(size_of::<VulkanImage>(), 48);
        assert_eq!(align_of::<VulkanImage>(), 8);
        assert_eq!(offset_of!(VulkanImage, image), 0);
        assert_eq!(offset_of!(VulkanImage, view), 8);
        assert_eq!(offset_of!(VulkanImage, state), 16);
        assert_eq!(offset_of!(VulkanImage, usage), 44);

        assert_eq!(size_of::<ReflectionGuideKind>(), 4);
        assert_eq!(size_of::<ReflectionGuide>(), 56);
        assert_eq!(align_of::<ReflectionGuide>(), 8);
        assert_eq!(offset_of!(ReflectionGuide, image), 0);
        assert_eq!(offset_of!(ReflectionGuide, kind), 48);
        assert_eq!(offset_of!(ReflectionGuide, reserved), 52);

        assert_eq!(size_of::<DlssRrResources>(), 392);
        assert_eq!(align_of::<DlssRrResources>(), 8);
        assert_eq!(offset_of!(DlssRrResources, reflection_guide), 336);

        assert_eq!(size_of::<DlssRrFrameConstants>(), 472);
        assert_eq!(align_of::<DlssRrFrameConstants>(), 4);
        assert_eq!(offset_of!(DlssRrFrameConstants, jitter_offset), 384);
        assert_eq!(offset_of!(DlssRrFrameConstants, frame_index), 464);
        assert_eq!(offset_of!(DlssRrFrameConstants, reset), 468);
    }

    #[test]
    fn specular_hit_distance_requires_r32_sfloat() {
        let invalid = VulkanImage {
            format: vk::Format::R16_SFLOAT.as_raw().cast_unsigned(),
            ..Default::default()
        };
        assert!(ReflectionGuide::specular_hit_distance(invalid).is_err());

        let valid = VulkanImage {
            format: vk::Format::R32_SFLOAT.as_raw().cast_unsigned(),
            ..Default::default()
        };
        let guide = ReflectionGuide::specular_hit_distance(valid).unwrap();
        assert_eq!(guide.kind(), ReflectionGuideKind::SpecularHitDistance);
    }

    #[test]
    fn project_identity_requires_non_empty_c_strings() {
        assert!(Streamline::validate_project_identity("", "1.0.0").is_err());
        assert!(Streamline::validate_project_identity("project", "").is_err());
        assert!(Streamline::validate_project_identity("project\0id", "1.0.0").is_err());
        assert!(Streamline::validate_project_identity("project", "1\0.0").is_err());
        Streamline::validate_project_identity("8e3b4a2d-44cc-4d62-9725-86e7412f4eb0", "1.0.0")
            .unwrap();
    }

    #[cfg(not(nvidia_streamline_native))]
    #[test]
    fn unsupported_target_keeps_stub_backend() {
        const { assert!(!NATIVE_BACKEND_AVAILABLE) };
        assert!(Streamline::staged_runtime_dir().is_none());
        assert!(Streamline::initialize("project", "1.0.0", ".").is_err());
        let streamline = Streamline {
            evaluator: Evaluator {
                context: Arc::new(backend::Context),
            },
            runtime_dir: PathBuf::new(),
            #[cfg(target_os = "windows")]
            interposer_path: PathBuf::new(),
            #[cfg(target_os = "windows")]
            proxy_created: false,
        };

        // Null is explicitly allowed and does not require an instance or loader.
        unsafe {
            assert!(!streamline.is_dlss_rr_supported(vk::PhysicalDevice::null()));
            assert!(
                !streamline
                    .evaluator()
                    .is_dlss_rr_supported(vk::PhysicalDevice::null())
            );
        }
    }

    #[cfg(all(target_os = "windows", not(nvidia_streamline_native)))]
    #[test]
    fn proxy_creation_failures_preserve_single_instance_guard() {
        let mut streamline = Streamline {
            evaluator: Evaluator {
                context: Arc::new(backend::Context),
            },
            runtime_dir: PathBuf::new(),
            interposer_path: PathBuf::new(),
            proxy_created: false,
        };
        let create_info = vk::InstanceCreateInfo::default();

        // No nested pointers or callbacks; the stub cannot create an owner.
        assert!(unsafe { streamline.create_proxy_ash_instance(&create_info) }.is_err());

        assert!(!streamline.proxy_created);

        #[cfg(feature = "vk-graph")]
        {
            // No nested pointers or callbacks; the stub cannot create an owner.
            assert!(unsafe { streamline.create_proxy_vulkan_instance(&create_info) }.is_err());

            assert!(!streamline.proxy_created);
        }

        streamline.proxy_created = true;

        // The guard rejects creation before accessing the pointer-free create info.
        let error = unsafe { streamline.create_proxy_ash_instance(&create_info) }
            .err()
            .unwrap();

        assert_eq!(
            error.to_string(),
            "a streamline proxy vulkan instance has already been created"
        );

        #[cfg(feature = "vk-graph")]
        {
            // The guard rejects creation before accessing the pointer-free create info.
            let error = unsafe { streamline.create_proxy_vulkan_instance(&create_info) }
                .err()
                .unwrap();

            assert_eq!(
                error.to_string(),
                "a streamline proxy vulkan instance has already been created"
            );
        }
    }
}
