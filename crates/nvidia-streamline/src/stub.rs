#[cfg(all(target_os = "windows", feature = "vk-graph"))]
use vk_graph::driver::instance::Instance;
use {
    crate::{DlssRrFrameConstants, DlssRrQuality, DlssRrResources},
    anyhow::{Result, bail},
    ash::vk,
    std::path::Path,
};

pub(crate) struct Context;

#[expect(clippy::unused_self, reason = "matches the native context interface")]
impl Context {
    pub(crate) fn initialize(
        _project_id: &str,
        _engine_version: &str,
        _runtime_dir: &Path,
    ) -> Result<Self> {
        bail!("native streamline is available only on x86_64-pc-windows-msvc")
    }

    pub(crate) unsafe fn is_dlss_rr_supported(&self, _physical_device: vk::PhysicalDevice) -> bool {
        false
    }

    pub(crate) fn optimal_render_extent(
        &self,
        _output_extent: [u32; 2],
        _quality: DlssRrQuality,
    ) -> Result<[u32; 2]> {
        bail!("native streamline is unavailable on this target")
    }

    pub(crate) unsafe fn evaluate_dlss_rr(
        &self,
        _command_buffer: vk::CommandBuffer,
        _frame: &DlssRrFrameConstants,
        _resources: &DlssRrResources,
        _quality: DlssRrQuality,
    ) -> Result<()> {
        bail!("native streamline is unavailable on this target")
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "matches the fallible native backend while preserving no-op shutdown"
    )]
    pub(crate) unsafe fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    pub(crate) fn is_active(&self) -> bool {
        false
    }

    #[cfg(target_os = "windows")]
    pub(crate) unsafe fn create_proxy_ash_instance(
        &self,
        _interposer_path: &Path,
        _create_info: &vk::InstanceCreateInfo<'_>,
    ) -> Result<ProxyOwner> {
        bail!("the streamline vulkan proxy is unavailable on this target")
    }

    #[cfg(all(target_os = "windows", feature = "vk-graph"))]
    pub(crate) unsafe fn create_proxy_vulkan_instance(
        &self,
        _interposer_path: &Path,
        _create_info: &vk::InstanceCreateInfo<'_>,
    ) -> Result<(Instance, ProxyOwner)> {
        bail!("the streamline vulkan proxy is unavailable on this target")
    }
}

#[cfg(target_os = "windows")]
pub(crate) struct ProxyOwner;

#[cfg(target_os = "windows")]
#[expect(
    clippy::unused_self,
    reason = "matches the native proxy owner interface"
)]
impl ProxyOwner {
    pub(crate) fn entry(&self) -> &ash::Entry {
        unreachable!("the streamline vulkan proxy is unavailable on this target")
    }

    pub(crate) fn instance(&self) -> &ash::Instance {
        unreachable!("the streamline vulkan proxy is unavailable on this target")
    }

    pub(crate) fn destroy(&mut self) {}
}

#[cfg(target_os = "windows")]
impl Drop for ProxyOwner {
    fn drop(&mut self) {}
}
