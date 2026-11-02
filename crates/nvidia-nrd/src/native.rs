use {
    crate::{Frame, Resources, validate_rectangles},
    anyhow::Context as _,
    ash::vk::{self, Handle as _},
    std::{ffi::c_void, ptr::NonNull, sync::Mutex},
};

#[cfg(feature = "vk-graph")]
use vk_graph::driver::device::Device;

static API_LOCK: Mutex<()> = Mutex::new(());

unsafe extern "C" {
    fn nvidia_nrd_create(
        instance: *mut c_void,
        physical_device: *mut c_void,
        device: *mut c_void,
        queue_family_index: u32,
        vulkan_minor_version: u8,
        queued_evaluations: u8,
        output: *mut *mut c_void,
    ) -> i32;
    fn nvidia_nrd_evaluate(
        context: *mut c_void,
        command_buffer: *mut c_void,
        frame: *const Frame,
        resources: *const Resources,
    ) -> i32;
    fn nvidia_nrd_destroy(context: *mut c_void);
}

pub struct Nrd {
    context: Option<NonNull<c_void>>,
    #[cfg(feature = "vk-graph")]
    device: Option<Device>,
}

impl Nrd {
    fn check_result(result: i32) -> anyhow::Result<()> {
        match result {
            0 => Ok(()),
            -1 => anyhow::bail!("nrd native exception"),
            -2 => anyhow::bail!("nrd invalid argument"),
            -3 => anyhow::bail!("nrd out of memory"),
            value => anyhow::bail!("nrd failed with result {value}"),
        }
    }

    /// Creates a runtime retaining the graph device until this runtime is dropped.
    ///
    /// # Errors
    /// Returns an error if the API version is unsupported or native context creation fails.
    ///
    /// # Safety
    ///
    /// The queue family, enabled extensions, and synchronization must satisfy
    /// [`Self::from_raw`]. Device ownership does not establish these requirements.
    #[cfg(feature = "vk-graph")]
    pub unsafe fn new(device: &Device, queue_family_index: u32) -> anyhow::Result<Self> {
        let instance = &device.physical.instance;
        let api_version = instance
            .info
            .api_version
            .to_vk_api_version()
            .min(device.physical.properties_v1_0.api_version);

        // The caller establishes queue/extension requirements. The retained graph device
        // keeps the borrowed Vulkan handles alive.
        let mut nrd = unsafe {
            Self::from_raw(
                instance.handle(),
                device.physical.handle,
                device.handle(),
                queue_family_index,
                api_version,
            )?
        };

        nrd.device = Some(device.clone());

        Ok(nrd)
    }

    /// Creates a runtime borrowing raw Vulkan handles without taking ownership.
    ///
    /// # Errors
    /// Returns an error if the API version is unsupported, handles cannot be represented
    /// by native pointers, or native context creation fails.
    ///
    /// # Safety
    ///
    /// The device must belong to the physical device and instance. All handles must remain
    /// valid until [`Self::shutdown`] returns. `queue_family_index` must identify a graphics
    /// and compute capable family with queue 0 created on the device; command buffers passed
    /// to [`Self::evaluate`] must belong to that family. The caller must externally synchronize
    /// queue access and NRD work. `api_version` must be standard Vulkan 1.3 or later,
    /// enabled for the instance and supported by the device. Enable the extensions and
    /// device features documented in [`Self::required_device_extensions`]. Unsupported
    /// API versions are rejected before accessing the handles.
    /// Complete all NRD GPU work before resolution changes or shutdown, and call
    /// shutdown before destroying the device or instance. Dropping alone leaks the native context.
    pub unsafe fn from_raw(
        instance: vk::Instance,
        physical_device: vk::PhysicalDevice,
        device: vk::Device,
        queue_family_index: u32,
        api_version: u32,
    ) -> anyhow::Result<Self> {
        Self::validate_api_version(api_version)?;

        let _guard = API_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut context = std::ptr::null_mut();

        let result = unsafe {
            nvidia_nrd_create(
                usize::try_from(instance.as_raw())? as *mut c_void,
                usize::try_from(physical_device.as_raw())? as *mut c_void,
                usize::try_from(device.as_raw())? as *mut c_void,
                queue_family_index,
                u8::try_from(vk::api_version_minor(api_version))?,
                crate::QUEUED_EVALUATIONS,
                &raw mut context,
            )
        };

        Self::check_result(result).context("creating nrd context")?;

        Ok(Self {
            context: Some(NonNull::new(context).context("nrd returned a null context")?),
            #[cfg(feature = "vk-graph")]
            device: None,
        })
    }

    /// Records RELAX SH work into an active command buffer.
    ///
    /// Inputs must declare `SHADER_READ_ONLY_OPTIMAL`, and outputs `GENERAL`.
    /// Other layouts return an error before recording work or changing runtime state.
    ///
    /// # Errors
    /// Returns an error for invalid layouts, a shut-down runtime, an unrepresentable
    /// command-buffer handle, or native evaluation failure. Pre-FFI errors do not
    /// consume slots; native failures after `NewFrame` can. Errors do not reliably
    /// indicate whether native advancement occurred. Follow the conservative
    /// retirement rule in [`crate::QUEUED_EVALUATIONS`] even when calls fail.
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
        command_buffer: vk::CommandBuffer,
        frame: &Frame,
        resources: &Resources,
    ) -> anyhow::Result<()> {
        resources.validate_layouts()?;
        validate_rectangles(
            [frame.resource_width, frame.resource_height],
            [frame.width, frame.height],
            [frame.previous_width, frame.previous_height],
        )?;

        let context = self.context.context("nrd has been shut down")?;
        let _guard = API_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let result = unsafe {
            nvidia_nrd_evaluate(
                context.as_ptr(),
                usize::try_from(command_buffer.as_raw())? as *mut c_void,
                frame,
                resources,
            )
        };

        Self::check_result(result).context("evaluating nrd relax sh")
    }

    /// Destroys the native context after all submitted work has completed.
    ///
    /// # Safety
    ///
    /// The Vulkan device must be idle with respect to every NRD dispatch.
    pub unsafe fn shutdown(&mut self) {
        let Some(context) = self.context.take() else {
            return;
        };

        let _guard = API_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        unsafe { nvidia_nrd_destroy(context.as_ptr()) };
    }
}

impl Drop for Nrd {
    fn drop(&mut self) {
        if self.context.is_some() && !std::thread::panicking() {
            log::error!("nrd dropped without gpu-safe shutdown; leaking the native context");
        }
    }
}

// Native API access is serialized by API_LOCK, including after moving between threads.
unsafe impl Send for Nrd {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_bridge_rejects_vulkan_1_2() {
        // Creation only stores these handles; rejection must not access Vulkan.
        let handle = NonNull::<c_void>::dangling().as_ptr();
        let mut context = std::ptr::null_mut();

        let result = unsafe {
            nvidia_nrd_create(
                handle,
                handle,
                handle,
                0,
                2,
                crate::QUEUED_EVALUATIONS,
                &raw mut context,
            )
        };

        if !context.is_null() {
            unsafe { nvidia_nrd_destroy(context) };
        }

        assert_eq!(result, -2);
    }

    #[test]
    fn native_bridge_uses_public_evaluation_capacity() {
        let handle = NonNull::<c_void>::dangling().as_ptr();
        let mut context = std::ptr::null_mut();

        assert_eq!(
            unsafe { nvidia_nrd_create(handle, handle, handle, 0, 3, 0, &raw mut context) },
            -2
        );
        assert!(context.is_null());

        assert_eq!(
            unsafe {
                nvidia_nrd_create(
                    handle,
                    handle,
                    handle,
                    0,
                    3,
                    crate::QUEUED_EVALUATIONS,
                    &raw mut context,
                )
            },
            0
        );
        assert!(!context.is_null());

        // Creation only stores handles; no Vulkan resources exist before evaluation.
        unsafe { nvidia_nrd_destroy(context) };
    }
}
