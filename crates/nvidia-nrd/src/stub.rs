#[cfg(feature = "vk-graph")]
use vk_graph::driver::device::Device;

use {
    crate::{Frame, Resources},
    ash::vk,
};

pub struct Nrd;

impl Nrd {
    /// Creates a runtime retaining the graph device until this runtime is dropped.
    ///
    /// # Errors
    /// Always returns an error because the native backend is unavailable.
    ///
    /// # Safety
    ///
    /// The queue family, enabled extensions, and synchronization must satisfy
    /// [`Self::from_raw`]. Device ownership does not establish these requirements.
    #[cfg(feature = "vk-graph")]
    pub unsafe fn new(_: &Device, _: u32) -> anyhow::Result<Self> {
        anyhow::bail!("nrd is unavailable on this target")
    }

    /// Creates a runtime borrowing raw Vulkan handles without taking ownership.
    ///
    /// # Errors
    /// Always returns an error because the native backend is unavailable.
    ///
    /// # Safety
    ///
    /// The device must belong to the physical device and instance. All handles must remain
    /// valid until [`Self::shutdown`] returns. `queue_family_index` must identify a graphics
    /// and compute capable family with queue 0 created on the device; command buffers passed
    /// to [`Self::evaluate`] must belong to that family. The caller must externally synchronize
    /// queue access and NRD work. `api_version` must be standard Vulkan 1.3 or later,
    /// enabled for the instance and supported by the device. Enable the extensions and
    /// device features documented in [`Self::required_device_extensions`].
    /// Complete all NRD GPU work before resolution changes or shutdown, and call
    /// shutdown before destroying the device or instance. Dropping alone leaks the native context.
    pub unsafe fn from_raw(
        _: vk::Instance,
        _: vk::PhysicalDevice,
        _: vk::Device,
        _: u32,
        _: u32,
    ) -> anyhow::Result<Self> {
        anyhow::bail!("nrd is unavailable on this target")
    }

    /// Records RELAX SH work into an active command buffer.
    ///
    /// Inputs must declare `SHADER_READ_ONLY_OPTIMAL`, and outputs `GENERAL`.
    ///
    /// # Errors
    /// Always returns an error because the native backend is unavailable; no slot
    /// is consumed. With a native backend, pre-FFI errors do not consume slots,
    /// but failures after `NewFrame` can. Errors do not reliably indicate native
    /// advancement; follow [`crate::QUEUED_EVALUATIONS`] even when calls fail.
    ///
    /// # Safety
    ///
    /// All images must belong to this instance's device and remain alive until execution completes.
    /// Transition them to their declared layouts and synchronize prior writes for compute shader
    /// sampled reads (inputs) or storage reads/writes (outputs) before NRD executes.
    /// NRD restores the declared layouts after its dispatches.
    /// Before each call, retire all older work according to
    /// [`crate::QUEUED_EVALUATIONS`], counting every call including errors, not
    /// `frame.frame_index`. Do not infer native slot indices from this count.
    /// The caller must synchronize shutdown and resolution changes with
    /// all previously submitted NRD work.
    /// The command buffer must be recording on the device and queue family supplied at creation.
    pub unsafe fn evaluate(
        &mut self,
        _: vk::CommandBuffer,
        _: &Frame,
        _: &Resources,
    ) -> anyhow::Result<()> {
        anyhow::bail!("nrd is unavailable on this target")
    }

    /// Destroys the native context after all submitted work has completed.
    ///
    /// # Safety
    ///
    /// The Vulkan device must be idle with respect to every NRD dispatch.
    pub unsafe fn shutdown(&mut self) {}
}
