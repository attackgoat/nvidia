#[cfg(feature = "vk-graph")]
use vk_graph::driver::instance::Instance;
use {
    crate::{DlssRrFrameConstants, DlssRrQuality, DlssRrResources, ffi},
    anyhow::{Context as _, Result, anyhow, ensure},
    ash::vk::{self, Handle as _},
    log::{error, info, warn},
    std::{
        ffi::{CStr, CString, c_char, c_void},
        os::windows::ffi::OsStrExt as _,
        path::Path,
        ptr::NonNull,
        sync::{
            Mutex,
            atomic::{AtomicBool, Ordering},
        },
    },
};

static API_LOCK: Mutex<()> = Mutex::new(());
static CONTEXT_ACTIVE: AtomicBool = AtomicBool::new(false);

pub(crate) struct Context {
    raw: Mutex<Option<NonNull<ffi::Context>>>,
}

impl Context {
    fn raw(&self) -> Option<NonNull<ffi::Context>> {
        *self
            .raw
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    // All Streamline GPU work must have completed.
    pub(crate) unsafe fn shutdown(&self) -> Result<()> {
        let _guard = lock_api();
        let raw = self
            .raw
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();

        let Some(raw) = raw else {
            return Ok(());
        };

        let result = unsafe { ffi::nvidia_sl_shutdown(raw.as_ptr()) };

        CONTEXT_ACTIVE.store(false, Ordering::SeqCst);

        if result != 0 {
            let message = format!("shutting down streamline: {}", result_name(result));

            error!("{message}");

            return Err(anyhow!(message));
        }

        Ok(())
    }

    pub(crate) fn initialize(
        project_id: &str,
        engine_version: &str,
        runtime_dir: &Path,
    ) -> Result<Self> {
        ensure!(
            runtime_dir.is_absolute(),
            "runtime directory for streamline must be absolute"
        );
        ensure!(
            runtime_dir.is_dir(),
            "runtime directory for streamline does not exist: {}",
            runtime_dir.display()
        );

        let interposer_path = runtime_dir.join("sl.interposer.dll");

        ensure!(
            interposer_path.is_file(),
            "interposer for streamline does not exist: {}",
            interposer_path.display()
        );

        let interposer_wide = wide_path(&interposer_path);
        let plugin_wide = wide_path(runtime_dir);
        let project_id = CString::new(project_id).context("converting nvidia project id")?;
        let engine_version =
            CString::new(engine_version).context("converting nvidia engine version")?;
        let _guard = lock_api();

        ensure!(
            !CONTEXT_ACTIVE.load(Ordering::SeqCst),
            "another streamline context is already active"
        );

        let mut raw = std::ptr::null_mut();

        let result = unsafe {
            ffi::nvidia_sl_init(
                interposer_wide.as_ptr(),
                plugin_wide.as_ptr(),
                project_id.as_ptr(),
                engine_version.as_ptr(),
                streamline_log,
                &raw mut raw,
            )
        };

        ensure_result("initializing streamline", result)?;
        let raw =
            NonNull::new(raw).context("initialization of streamline returned a null context")?;
        CONTEXT_ACTIVE.store(true, Ordering::SeqCst);

        info!("initialized streamline 2.9.0 dlss ray reconstruction");

        Ok(Self {
            raw: Mutex::new(Some(raw)),
        })
    }

    // The caller must uphold Evaluator::is_dlss_rr_supported's handle and loader lifetimes.
    pub(crate) unsafe fn is_dlss_rr_supported(&self, physical_device: vk::PhysicalDevice) -> bool {
        if physical_device == vk::PhysicalDevice::null() {
            return false;
        }

        let _guard = lock_api();

        let Some(raw) = self.raw() else {
            return false;
        };

        let result = unsafe {
            ffi::nvidia_sl_is_dlss_rr_supported(
                raw.as_ptr(),
                physical_device.as_raw() as *mut c_void,
            )
        };

        if result == 0 {
            info!("selected adapter supports dlss ray reconstruction");

            true
        } else {
            warn!(
                "selected adapter does not support dlss ray reconstruction: {}",
                result_name(result)
            );

            false
        }
    }

    pub(crate) fn optimal_render_extent(
        &self,
        output_extent: [u32; 2],
        quality: DlssRrQuality,
    ) -> Result<[u32; 2]> {
        ensure!(
            output_extent[0] > 0 && output_extent[1] > 0,
            "output extent must be nonzero"
        );

        let _guard = lock_api();
        let raw = self
            .raw()
            .context("context for streamline has already been shut down")?;
        let mut output = ffi::Extent {
            width: 0,
            height: 0,
        };

        let result = unsafe {
            ffi::nvidia_sl_dlss_rr_optimal_settings(
                raw.as_ptr(),
                output_extent[0],
                output_extent[1],
                quality as u32,
                &raw mut output,
            )
        };

        ensure_result("querying dlss-rr optimal settings", result)?;

        ensure!(
            output.width > 0 && output.height > 0,
            "query through streamline returned a zero render extent"
        );

        Ok([output.width, output.height])
    }

    // The caller must uphold Evaluator::evaluate_dlss_rr's device and GPU lifetime requirements.
    pub(crate) unsafe fn evaluate_dlss_rr(
        &self,
        command_buffer: vk::CommandBuffer,
        frame: &DlssRrFrameConstants,
        resources: &DlssRrResources,
        quality: DlssRrQuality,
    ) -> Result<()> {
        ensure!(
            command_buffer != vk::CommandBuffer::null(),
            "command buffer must not be null"
        );

        let _guard = lock_api();
        let raw = self
            .raw()
            .context("context for streamline has already been shut down")?;

        let result = unsafe {
            ffi::nvidia_sl_evaluate_dlss_rr(
                raw.as_ptr(),
                command_buffer.as_raw() as *mut c_void,
                quality as u32,
                std::ptr::from_ref(frame),
                std::ptr::from_ref(resources),
            )
        };

        ensure_result("evaluating dlss ray reconstruction", result)
    }

    pub(crate) fn is_active(&self) -> bool {
        self.raw().is_some()
    }

    // The caller must uphold Streamline::create_proxy_ash_instance's pointer and owner contract.
    pub(crate) unsafe fn create_proxy_ash_instance(
        &self,
        interposer_path: &Path,
        create_info: &vk::InstanceCreateInfo<'_>,
    ) -> Result<ProxyOwner> {
        let _guard = lock_api();

        ensure!(
            self.raw().is_some(),
            "context for streamline has been shut down"
        );

        let entry = unsafe { ash::Entry::load_from(interposer_path) }
            .context("loading vulkan through the streamline interposer")?;

        let instance = unsafe { entry.create_instance(create_info, None) }
            .context("creating a vulkan instance through streamline")?;

        Ok(ProxyOwner {
            entry: Some(entry),
            instance: Some(instance),
        })
    }

    #[cfg(feature = "vk-graph")]
    // The caller must uphold Streamline::create_proxy_vulkan_instance's pointer and owner contract.
    pub(crate) unsafe fn create_proxy_vulkan_instance(
        &self,
        interposer_path: &Path,
        create_info: &vk::InstanceCreateInfo<'_>,
    ) -> Result<(Instance, ProxyOwner)> {
        let mut owner = unsafe { self.create_proxy_ash_instance(interposer_path, create_info)? };

        let instance =
            match Instance::try_from_entry(owner.entry().clone(), owner.instance().handle()) {
                Ok(instance) => instance,
                Err(error) => {
                    owner.destroy();

                    return Err(anyhow::Error::new(error)
                        .context("importing the streamline vulkan instance"));
                }
            };

        Ok((instance, owner))
    }
}

// Native calls are serialized by API_LOCK; access to the context pointer is mutex-protected.
unsafe impl Send for Context {}

// Shared access follows the same locking protocol as Send.
unsafe impl Sync for Context {}

pub(crate) struct ProxyOwner {
    entry: Option<ash::Entry>,
    instance: Option<ash::Instance>,
}

impl ProxyOwner {
    pub(crate) fn entry(&self) -> &ash::Entry {
        self.entry.as_ref().expect("proxy vulkan loader invariant")
    }

    pub(crate) fn instance(&self) -> &ash::Instance {
        self.instance
            .as_ref()
            .expect("proxy vulkan instance invariant")
    }

    pub(crate) fn destroy(&mut self) {
        if let Some(instance) = self.instance.take() {
            unsafe { instance.destroy_instance(None) };
        }

        self.entry.take();
    }
}

impl Drop for ProxyOwner {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.instance.take();

            if let Some(entry) = self.entry.take() {
                std::mem::forget(entry);
            }

            return;
        }

        self.destroy();
    }
}

fn ensure_result(operation: &str, result: i32) -> Result<()> {
    ensure!(result == 0, "{operation}: {}", result_name(result));

    Ok(())
}

fn result_name(result: i32) -> String {
    let name = unsafe { ffi::nvidia_sl_result_name(result) };

    if name.is_null() {
        return format!("result {result}");
    }

    format!(
        "{} ({result})",
        unsafe { CStr::from_ptr(name) }.to_string_lossy()
    )
}

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

fn lock_api() -> std::sync::MutexGuard<'static, ()> {
    API_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

unsafe extern "C" fn streamline_log(message_type: i32, message: *const c_char) {
    if message.is_null() {
        return;
    }

    let _ = std::panic::catch_unwind(|| {
        let message = unsafe { CStr::from_ptr(message) }.to_string_lossy();

        match message_type {
            0 => info!(target: "streamline", "{message}"),
            1 => warn!(target: "streamline", "{message}"),
            _ => error!(target: "streamline", "{message}"),
        }
    });
}
