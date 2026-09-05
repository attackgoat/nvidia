#[cfg(feature = "vk-graph")]
use vk_graph::driver::{device::Device, instance::Instance};
use {
    crate::{
        DlssRrConfig, DlssRrFrameConstants, DlssRrQuality, DlssRrResources, InitInfo,
        VulkanInitInfo,
    },
    ash::vk,
    std::{ffi::CString, path::Path},
};

#[derive(Clone)]
pub struct Evaluator;

impl Evaluator {
    #[must_use]
    pub fn is_active(&self) -> bool {
        false
    }

    /// # Errors
    /// Always returns an error because the native backend is unavailable.
    pub fn optimal_render_extent(
        &self,
        _output_extent: [u32; 2],
        _quality: DlssRrQuality,
    ) -> anyhow::Result<[u32; 2]> {
        anyhow::bail!("ngx is available only on x86_64-unknown-linux-gnu")
    }

    /// # Safety
    ///
    /// This evaluator and its owning NGX context must remain valid until execution completes. The
    /// command buffer must be recording on the context's device; every image and view must belong
    /// to that device, and each declared layout must match its actual state. Image ranges,
    /// formats, and extents must describe their views accurately. Before evaluation,
    /// transition all inputs (including depth and the reflection guide) to
    /// `SHADER_READ_ONLY_OPTIMAL` and the output to `GENERAL`. Evaluation leaves these
    /// layouts unchanged; this wrapper does not record layout transitions. All handles must
    /// remain valid until GPU execution completes. If dimensions or quality change, prior
    /// evaluations must have completed first.
    ///
    /// # Errors
    /// Always returns an error because the native backend is unavailable.
    pub unsafe fn evaluate(
        &self,
        _command_buffer: vk::CommandBuffer,
        _frame: &DlssRrFrameConstants,
        _resources: &DlssRrResources,
        _quality: DlssRrQuality,
    ) -> anyhow::Result<()> {
        anyhow::bail!("ngx is available only on x86_64-unknown-linux-gnu")
    }

    /// # Safety
    ///
    /// All NGX GPU work must have completed before shutdown.
    ///
    /// # Errors
    /// This stub always succeeds; the native backend can report shutdown failures.
    pub unsafe fn shutdown(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

pub struct Ngx;

impl Ngx {
    /// Returns whether this build includes the native NGX backend.
    #[must_use]
    pub const fn native_backend_available() -> bool {
        false
    }

    /// Returns the build-staged runtime directory, or `None` without the native backend.
    #[must_use]
    pub fn staged_runtime_dir() -> Option<&'static Path> {
        None
    }

    /// # Errors
    /// Always returns an error because the native backend is unavailable.
    pub fn new(_info: InitInfo) -> anyhow::Result<Self> {
        anyhow::bail!("ngx is available only on x86_64-unknown-linux-gnu")
    }

    #[must_use]
    pub fn instance_extensions(&self) -> &[CString] {
        &[]
    }

    /// Queries NGX device extension requirements.
    ///
    /// # Safety
    ///
    /// `physical_device` must be a valid physical device enumerated from `instance`.
    /// The instance, physical device, and Vulkan loader that created them must remain
    /// valid throughout the call and must not be destroyed concurrently.
    ///
    /// # Errors
    /// Always returns an error because the native backend is unavailable.
    #[cfg(feature = "vk-graph")]
    pub unsafe fn device_extensions(
        &self,
        _instance: &Instance,
        _physical_device: vk::PhysicalDevice,
    ) -> anyhow::Result<Vec<CString>> {
        anyhow::bail!("ngx is available only on x86_64-unknown-linux-gnu")
    }

    /// # Safety
    ///
    /// `instance` must be a valid Vulkan instance, and `physical_device` must be a valid
    /// physical device enumerated from it. The instance, physical device, and Vulkan loader
    /// that created them must remain valid throughout the call and must not be destroyed
    /// concurrently.
    ///
    /// # Errors
    /// Always returns an error because the native backend is unavailable.
    pub unsafe fn device_extensions_raw(
        &self,
        _instance: vk::Instance,
        _physical_device: vk::PhysicalDevice,
    ) -> anyhow::Result<Vec<CString>> {
        anyhow::bail!("ngx is available only on x86_64-unknown-linux-gnu")
    }

    /// Initializes NGX while retaining ownership of the graph device.
    ///
    /// # Safety
    ///
    /// The instance and device must enable the extensions returned by NGX. Complete all
    /// NGX GPU work before explicit shutdown through `Ngx` or any evaluator. Dropping
    /// these wrappers does not shut down NGX.
    ///
    /// # Errors
    /// Always returns an error because the native backend is unavailable.
    #[cfg(feature = "vk-graph")]
    pub unsafe fn initialize(
        &mut self,
        _device: &Device,
        _config: DlssRrConfig,
    ) -> anyhow::Result<Evaluator> {
        anyhow::bail!("ngx is available only on x86_64-unknown-linux-gnu")
    }

    /// Initializes NGX without retaining ownership of the Vulkan device.
    ///
    /// # Safety
    ///
    /// Handles must be valid and belong to the same Vulkan instance and physical device.
    /// The instance and device must enable the extensions returned by NGX. The entrypoints
    /// must belong to the loader that created these handles. Keep the loader, instance, and device alive
    /// until explicit shutdown through `Ngx` or any evaluator completes, and complete all
    /// NGX GPU work before shutdown. Dropping these wrappers does not shut down NGX.
    ///
    /// # Errors
    /// Always returns an error because the native backend is unavailable.
    pub unsafe fn initialize_raw(
        &mut self,
        _info: VulkanInitInfo,
        _config: DlssRrConfig,
    ) -> anyhow::Result<Evaluator> {
        anyhow::bail!("ngx is available only on x86_64-unknown-linux-gnu")
    }

    #[must_use]
    pub fn evaluator(&self) -> Option<Evaluator> {
        None
    }

    /// # Safety
    ///
    /// All NGX GPU work must have completed before shutdown.
    ///
    /// # Errors
    /// This stub always succeeds; the native backend can report shutdown failures.
    pub unsafe fn shutdown(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}
