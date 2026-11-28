#[cfg(feature = "vk-graph")]
use super::Instance;
use {
    super::{API_LOCK, Evaluator, EvaluatorInner, NativeContext, Ngx, check_result},
    crate::{DlssFgConfig, DlssFgResources, DlssRrFrameConstants},
    anyhow::Context as _,
    ash::vk::{self, Handle as _},
    std::{
        ffi::{CString, c_void},
        sync::{Arc, atomic::Ordering},
    },
};

/// One native Frame Generation feature and its parameter map. The shared NGX
/// context retains these until [`Self::release`] or NGX shutdown. Dropping this
/// evaluator does not release them, since only the caller knows when the GPU
/// no longer references the feature.
#[must_use]
pub struct DlssFgEvaluator {
    inner: Arc<EvaluatorInner>,
    config: DlssFgConfig,
    active: bool,
}

impl DlssFgEvaluator {
    /// Records one intermediate frame between the previous and current real frames.
    /// A successful recording can still produce a real-frame copy; read the
    /// suppression buffer after the evaluation fence and host-read synchronization.
    ///
    /// Use unjittered camera matrices (row-major for row vectors) and supply the
    /// jitter offset in clip-space units. As for RR, `mvec_scale` converts stored
    /// vectors to guide-pixel displacement; use `[1.0, 1.0]` for pixel motion.
    /// This binding interprets a zero scale axis as unity. Set `reset` on first
    /// use and after any discontinuity.
    ///
    /// # Safety
    /// The command buffer must be recording on the context's device, outside
    /// a render pass. Creation and evaluations must execute in order on the
    /// same graphics-capable queue. Supply valid image handles, matching
    /// views, layouts and usages, and the required suppression buffer.
    /// Synchronize access to the resources; NGX does not supply the application's
    /// external barriers or layout transitions. Keep the feature, its context,
    /// and all resources alive until the GPU completes the work.
    ///
    /// # Errors
    /// Returns an error for invalid resource declarations, a released feature
    /// or context, or an NGX recording failure. Discard a failed recording;
    /// reset or recreate the feature before attempting interpolation again.
    pub unsafe fn evaluate(
        &mut self,
        command_buffer: vk::CommandBuffer,
        frame: &DlssRrFrameConstants,
        resources: &DlssFgResources,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.active,
            "ngx frame generation feature has been released"
        );
        anyhow::ensure!(
            command_buffer != vk::CommandBuffer::null(),
            "null vulkan command buffer"
        );
        anyhow::ensure!(
            frame.mvec_scale.iter().all(|scale| scale.is_finite()),
            "non-finite dlss frame generation motion scale"
        );
        resources.validate(self.config)?;

        let _guard = API_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let context = self.inner.context.load(Ordering::SeqCst);
        anyhow::ensure!(!context.is_null(), "ngx evaluator has been shut down");

        check_result(unsafe {
            nvidia_dlss_ngx_fg_evaluate(
                context,
                usize::try_from(command_buffer.as_raw())? as *mut c_void,
                frame,
                resources,
            )
        })
        .context("recording ngx dlss frame generation")
    }

    /// Releases the feature and its parameter map. The shared NGX context
    /// stays initialized for RR. Releasing again after success is a no-op.
    ///
    /// # Safety
    /// Wait for completion of every submission using the feature, including
    /// its creation commands. Reset or discard any unsubmitted command buffers
    /// that reference it; none may subsequently be submitted.
    ///
    /// # Errors
    /// Returns an error if NGX fails to release the feature or parameter map.
    /// A failed release can be retried with the same GPU completion guarantees.
    pub unsafe fn release(&mut self) -> anyhow::Result<()> {
        if !self.active {
            return Ok(());
        }

        let _guard = API_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let context = self.inner.context.load(Ordering::SeqCst);
        if !context.is_null() {
            check_result(unsafe { nvidia_dlss_ngx_fg_release(context) })
                .context("releasing ngx dlss frame generation")?;
        }

        self.active = false;
        Ok(())
    }
}

impl Evaluator {
    /// Queries `FrameGeneration.Available` on the initialized NGX device.
    /// Returns `false` if NGX reports the feature unavailable.
    ///
    /// # Errors
    /// Returns an error if the context has shut down or the query fails.
    pub fn frame_generation_available(&self) -> anyhow::Result<bool> {
        let _guard = API_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let context = self.inner.context.load(Ordering::SeqCst);
        anyhow::ensure!(!context.is_null(), "ngx evaluator has been shut down");

        let mut available = 0;
        check_result(unsafe { nvidia_dlss_ngx_fg_available(context, &raw mut available) })
            .context("querying ngx dlss frame generation capability")?;
        Ok(available != 0)
    }

    /// Creates the feature synchronously and records any initialization commands.
    /// One Frame Generation feature can be active in the shared NGX context.
    ///
    /// # Safety
    /// The instance and device must enable the requirements returned by FG
    /// discovery. The command buffer must be recording outside a render pass on
    /// the NGX device. Submit creation on the same graphics-capable queue before
    /// evaluations (or in the same command buffer). Follow
    /// [`DlssFgEvaluator::release`] before destroying or replacing the feature.
    ///
    /// # Errors
    /// Returns an error for invalid configuration, unavailable capability,
    /// an active feature, or an SDK allocation/creation failure.
    /// Discard the command buffer if creation fails.
    pub unsafe fn create_frame_generation(
        &self,
        command_buffer: vk::CommandBuffer,
        config: DlssFgConfig,
    ) -> anyhow::Result<DlssFgEvaluator> {
        anyhow::ensure!(
            command_buffer != vk::CommandBuffer::null(),
            "null vulkan command buffer"
        );
        config.validate()?;

        let _guard = API_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let context = self.inner.context.load(Ordering::SeqCst);
        anyhow::ensure!(!context.is_null(), "ngx evaluator has been shut down");
        check_result(unsafe {
            nvidia_dlss_ngx_fg_create(
                context,
                usize::try_from(command_buffer.as_raw())? as *mut c_void,
                &raw const config,
            )
        })
        .context("creating ngx dlss frame generation")?;

        Ok(DlssFgEvaluator {
            inner: Arc::clone(&self.inner),
            config,
            active: true,
        })
    }
}

impl Ngx {
    /// Queries Frame Generation extension requirements before instance creation.
    ///
    /// # Errors
    /// Returns an error if discovery fails.
    pub fn frame_generation_instance_extensions(&self) -> anyhow::Result<Vec<CString>> {
        Self::query_instance_extensions(&self.info, true)
            .context("querying ngx dlss frame generation instance extensions")
    }

    /// Queries Frame Generation device requirements for the selected adapter.
    ///
    /// # Safety
    /// The physical device must have been enumerated from the live instance.
    ///
    /// # Errors
    /// Returns an error if the adapter does not support the feature or discovery fails.
    #[cfg(feature = "vk-graph")]
    pub unsafe fn frame_generation_device_extensions(
        &self,
        instance: &Instance,
        physical_device: vk::PhysicalDevice,
    ) -> anyhow::Result<Vec<CString>> {
        unsafe { self.frame_generation_device_extensions_raw(instance.handle(), physical_device) }
    }

    /// Queries Frame Generation device requirements for the selected adapter.
    ///
    /// # Safety
    /// The physical device must have been enumerated from the live instance;
    /// the Vulkan loader that created it must remain loaded.
    ///
    /// # Errors
    /// Returns an error if the adapter does not support the feature or discovery fails.
    pub unsafe fn frame_generation_device_extensions_raw(
        &self,
        instance: vk::Instance,
        physical_device: vk::PhysicalDevice,
    ) -> anyhow::Result<Vec<CString>> {
        unsafe { self.query_device_extensions(instance, physical_device, true) }
            .context("querying ngx dlss frame generation device extensions")
    }
}

unsafe extern "C" {
    fn nvidia_dlss_ngx_fg_available(context: *mut NativeContext, available: *mut i32) -> i32;
    fn nvidia_dlss_ngx_fg_create(
        context: *mut NativeContext,
        command_buffer: *mut c_void,
        config: *const DlssFgConfig,
    ) -> i32;
    fn nvidia_dlss_ngx_fg_evaluate(
        context: *mut NativeContext,
        command_buffer: *mut c_void,
        frame: *const DlssRrFrameConstants,
        resources: *const DlssFgResources,
    ) -> i32;
    fn nvidia_dlss_ngx_fg_release(context: *mut NativeContext) -> i32;
}
