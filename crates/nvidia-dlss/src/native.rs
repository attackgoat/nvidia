#[cfg(feature = "vk-graph")]
use vk_graph::driver::{device::Device, instance::Instance};
use {
    crate::{
        DLSS_RUNTIME_FILE, DlssRrConfig, DlssRrFrameConstants, DlssRrQuality, DlssRrResources,
        IdentityKind, InitInfo, NativeDlssRrResources, VulkanInitInfo,
    },
    anyhow::Context as _,
    ash::vk::{self, Handle as _},
    std::{
        ffi::{CStr, CString, c_char, c_void},
        os::unix::ffi::OsStrExt as _,
        path::Path,
        ptr::NonNull,
        sync::{
            Arc, Mutex,
            atomic::{AtomicPtr, Ordering},
        },
    },
};

const MAX_EXTENSIONS: usize = 32;
const MAX_EXTENSION_NAME_SIZE: usize = 256;

static ACTIVE_CONTEXT: AtomicPtr<NativeContext> = AtomicPtr::new(std::ptr::null_mut());
static API_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone)]
pub struct Evaluator {
    inner: Arc<EvaluatorInner>,
}

impl Evaluator {
    #[must_use]
    pub fn is_active(&self) -> bool {
        !self.inner.context.load(Ordering::SeqCst).is_null()
    }

    /// # Errors
    /// Returns an error for zero dimensions, a shut-down evaluator, or an NGX query failure.
    pub fn optimal_render_extent(
        &self,
        output_extent: [u32; 2],
        quality: DlssRrQuality,
    ) -> anyhow::Result<[u32; 2]> {
        anyhow::ensure!(
            output_extent[0] > 0 && output_extent[1] > 0,
            "zero output extent"
        );

        let _guard = API_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let context = self.inner.context.load(Ordering::SeqCst);

        anyhow::ensure!(!context.is_null(), "ngx evaluator has been shut down");

        let mut output = NativeExtent {
            width: 0,
            height: 0,
        };

        let result = unsafe {
            nvidia_dlss_ngx_optimal_settings(
                context,
                output_extent[0],
                output_extent[1],
                quality as u32,
                &raw mut output,
            )
        };

        check_result(result).context("querying ngx optimal render extent")?;

        anyhow::ensure!(output.width > 0 && output.height > 0, "zero render extent");

        Ok([output.width, output.height])
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
    /// Returns an error for a null command buffer, invalid resource declarations,
    /// a shut-down evaluator, or an NGX creation/evaluation failure.
    pub unsafe fn evaluate(
        &self,
        command_buffer: vk::CommandBuffer,
        frame: &DlssRrFrameConstants,
        resources: &DlssRrResources,
        quality: DlssRrQuality,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            command_buffer != vk::CommandBuffer::null(),
            "null vulkan command buffer"
        );

        let resources = resources.native(self.inner.config)?;

        anyhow::ensure!(
            quality != DlssRrQuality::Dlaa
                || resources.input_color.extent() == resources.output_color.extent(),
            "dlaa requires equal render and output dimensions"
        );

        let _guard = API_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let context = self.inner.context.load(Ordering::SeqCst);

        anyhow::ensure!(!context.is_null(), "ngx evaluator has been shut down");

        let result = unsafe {
            nvidia_dlss_ngx_evaluate(
                context,
                usize::try_from(command_buffer.as_raw())? as *mut c_void,
                quality as u32,
                frame,
                &raw const resources,
            )
        };

        check_result(result).context("evaluating ngx dlss-rr")
    }

    /// # Safety
    ///
    /// All NGX GPU work must have completed before shutdown.
    ///
    /// # Errors
    /// Returns an error if NGX resource release or shutdown fails.
    pub unsafe fn shutdown(&self) -> anyhow::Result<()> {
        self.inner.shutdown()
    }
}

struct EvaluatorInner {
    config: DlssRrConfig,
    context: AtomicPtr<NativeContext>,
    #[cfg(feature = "vk-graph")]
    _device: Option<Device>,
}

impl EvaluatorInner {
    fn shutdown(&self) -> anyhow::Result<()> {
        let _guard = API_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let context = self.context.swap(std::ptr::null_mut(), Ordering::SeqCst);

        if context.is_null() {
            return Ok(());
        }

        let _ = ACTIVE_CONTEXT.compare_exchange(
            context,
            std::ptr::null_mut(),
            Ordering::SeqCst,
            Ordering::SeqCst,
        );

        let result = unsafe { nvidia_dlss_ngx_shutdown(context) };

        check_result(result).context("shutting down ngx")
    }
}

#[repr(C)]
struct NativeContext {
    _private: [u8; 0],
}

#[repr(C)]
struct NativeExtensions {
    count: u32,
    names: [[c_char; MAX_EXTENSION_NAME_SIZE]; MAX_EXTENSIONS],
}

impl NativeExtensions {
    fn extension_names(&self) -> anyhow::Result<Vec<CString>> {
        let count = usize::try_from(self.count).context("converting ngx extension count")?;

        anyhow::ensure!(
            count <= MAX_EXTENSIONS,
            "invalid ngx extension count {count}"
        );

        self.names[..count]
            .iter()
            .map(|name| {
                let name = unsafe { CStr::from_ptr(name.as_ptr()) };

                CString::new(name.to_bytes()).context("copying ngx extension name")
            })
            .collect()
    }
}

impl Default for NativeExtensions {
    fn default() -> Self {
        Self {
            count: 0,
            names: [[0; MAX_EXTENSION_NAME_SIZE]; MAX_EXTENSIONS],
        }
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
struct NativeExtent {
    width: u32,
    height: u32,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct NativeIdentity {
    application_id: u64,
    project_id: *const c_char,
    engine_version: *const c_char,
}

unsafe extern "C" {
    fn nvidia_dlss_ngx_instance_extensions(
        identity: *const NativeIdentity,
        application_data_path: *const c_char,
        runtime_path: *const c_char,
        output: *mut NativeExtensions,
    ) -> i32;
    fn nvidia_dlss_ngx_device_extensions(
        identity: *const NativeIdentity,
        application_data_path: *const c_char,
        runtime_path: *const c_char,
        instance: *mut c_void,
        physical_device: *mut c_void,
        output: *mut NativeExtensions,
    ) -> i32;
    fn nvidia_dlss_ngx_init(
        identity: *const NativeIdentity,
        application_data_path: *const c_char,
        runtime_path: *const c_char,
        instance: *mut c_void,
        physical_device: *mut c_void,
        device: *mut c_void,
        get_instance_proc_addr: vk::PFN_vkGetInstanceProcAddr,
        get_device_proc_addr: vk::PFN_vkGetDeviceProcAddr,
        config: *const DlssRrConfig,
        output: *mut *mut NativeContext,
    ) -> i32;
    fn nvidia_dlss_ngx_optimal_settings(
        context: *mut NativeContext,
        output_width: u32,
        output_height: u32,
        quality: u32,
        output: *mut NativeExtent,
    ) -> i32;
    fn nvidia_dlss_ngx_evaluate(
        context: *mut NativeContext,
        command_buffer: *mut c_void,
        quality: u32,
        frame: *const DlssRrFrameConstants,
        resources: *const NativeDlssRrResources,
    ) -> i32;
    fn nvidia_dlss_ngx_shutdown(context: *mut NativeContext) -> i32;
    fn nvidia_dlss_ngx_result_name(result: i32) -> *const c_char;
}

pub struct Ngx {
    info: PreparedInfo,
    instance_extensions: Box<[CString]>,
    evaluator: Option<Evaluator>,
}

impl Ngx {
    /// Returns whether this build includes the native NGX backend.
    #[must_use]
    pub const fn native_backend_available() -> bool {
        true
    }

    /// Returns the build-staged runtime directory, or `None` without the native backend.
    #[must_use]
    pub fn staged_runtime_dir() -> Option<&'static Path> {
        Some(Path::new(env!("NVIDIA_DLSS_RUNTIME_DIR")))
    }

    /// # Errors
    /// Returns an error for invalid paths, missing runtime files, filesystem errors,
    /// or NGX extension discovery failure.
    pub fn new(info: InitInfo) -> anyhow::Result<Self> {
        let info = PreparedInfo::new(info)?;
        let identity = info.identity();
        let mut output = NativeExtensions::default();
        let _guard = API_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let result = unsafe {
            nvidia_dlss_ngx_instance_extensions(
                &raw const identity,
                info.application_data_path.as_ptr(),
                info.runtime_path.as_ptr(),
                &raw mut output,
            )
        };

        check_result(result).context("querying ngx instance extensions")?;

        Ok(Self {
            info,
            instance_extensions: output.extension_names()?.into_boxed_slice(),
            evaluator: None,
        })
    }

    #[must_use]
    pub fn instance_extensions(&self) -> &[CString] {
        &self.instance_extensions
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
    /// Returns an error if NGX rejects the device or extension discovery fails.
    #[cfg(feature = "vk-graph")]
    pub unsafe fn device_extensions(
        &self,
        instance: &Instance,
        physical_device: vk::PhysicalDevice,
    ) -> anyhow::Result<Vec<CString>> {
        unsafe { self.device_extensions_raw(instance.handle(), physical_device) }
    }

    /// # Safety
    ///
    /// `instance` must be a valid Vulkan instance, and `physical_device` must be a valid
    /// physical device enumerated from it. The instance, physical device, and Vulkan loader
    /// that created them must remain valid throughout the call and must not be destroyed
    /// concurrently.
    ///
    /// # Errors
    /// Returns an error if NGX rejects the handles/device or extension discovery fails.
    pub unsafe fn device_extensions_raw(
        &self,
        instance: vk::Instance,
        physical_device: vk::PhysicalDevice,
    ) -> anyhow::Result<Vec<CString>> {
        let identity = self.info.identity();
        let mut output = NativeExtensions::default();
        let _guard = API_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let result = unsafe {
            nvidia_dlss_ngx_device_extensions(
                &raw const identity,
                self.info.application_data_path.as_ptr(),
                self.info.runtime_path.as_ptr(),
                usize::try_from(instance.as_raw())? as *mut c_void,
                usize::try_from(physical_device.as_raw())? as *mut c_void,
                &raw mut output,
            )
        };

        check_result(result).context("querying ngx device extensions")?;

        output.extension_names()
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
    /// Returns an error if a context is already active or NGX initialization fails.
    #[cfg(feature = "vk-graph")]
    pub unsafe fn initialize(
        &mut self,
        device: &Device,
        config: DlssRrConfig,
    ) -> anyhow::Result<Evaluator> {
        let instance = &device.physical.instance;
        let info = VulkanInitInfo {
            instance: instance.handle(),
            physical_device: device.physical.handle,
            device: device.handle(),
            get_instance_proc_addr: Instance::entry(instance).static_fn().get_instance_proc_addr,
            get_device_proc_addr: instance.fp_v1_0().get_device_proc_addr,
        };

        unsafe { self.initialize_vulkan(&info, config, Some(device.clone())) }
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
    /// Returns an error if a context is already active or NGX initialization fails.
    pub unsafe fn initialize_raw(
        &mut self,
        info: VulkanInitInfo,
        config: DlssRrConfig,
    ) -> anyhow::Result<Evaluator> {
        unsafe {
            self.initialize_vulkan(
                &info,
                config,
                #[cfg(feature = "vk-graph")]
                None,
            )
        }
    }

    unsafe fn initialize_vulkan(
        &mut self,
        info: &VulkanInitInfo,
        config: DlssRrConfig,
        #[cfg(feature = "vk-graph")] device: Option<Device>,
    ) -> anyhow::Result<Evaluator> {
        anyhow::ensure!(
            !self.evaluator.as_ref().is_some_and(Evaluator::is_active),
            "ngx is already initialized"
        );

        self.evaluator = None;

        let _guard = API_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        anyhow::ensure!(
            ACTIVE_CONTEXT.load(Ordering::SeqCst).is_null(),
            "an ngx context is already active"
        );

        let identity = self.info.identity();
        let mut context = std::ptr::null_mut();

        let result = unsafe {
            nvidia_dlss_ngx_init(
                &raw const identity,
                self.info.application_data_path.as_ptr(),
                self.info.runtime_path.as_ptr(),
                usize::try_from(info.instance.as_raw())? as *mut c_void,
                usize::try_from(info.physical_device.as_raw())? as *mut c_void,
                usize::try_from(info.device.as_raw())? as *mut c_void,
                info.get_instance_proc_addr,
                info.get_device_proc_addr,
                &raw const config,
                &raw mut context,
            )
        };

        check_result(result).context("initializing ngx")?;
        let context = NonNull::new(context).context("ngx returned a null context")?;
        let context = context.as_ptr();
        ACTIVE_CONTEXT.store(context, Ordering::SeqCst);

        let evaluator = Evaluator {
            inner: Arc::new(EvaluatorInner {
                config,
                context: AtomicPtr::new(context),
                #[cfg(feature = "vk-graph")]
                _device: device,
            }),
        };
        self.evaluator = Some(evaluator.clone());
        Ok(evaluator)
    }

    #[must_use]
    pub fn evaluator(&self) -> Option<Evaluator> {
        self.evaluator
            .as_ref()
            .filter(|evaluator| evaluator.is_active())
            .cloned()
    }

    /// # Safety
    ///
    /// All NGX GPU work must have completed before shutdown.
    ///
    /// # Errors
    /// Returns an error if NGX resource release or shutdown fails.
    pub unsafe fn shutdown(&mut self) -> anyhow::Result<()> {
        match self.evaluator.take() {
            Some(evaluator) => unsafe { evaluator.shutdown() },
            None => Ok(()),
        }
    }
}

struct PreparedInfo {
    application_data_path: CString,
    runtime_path: CString,
    application_id: u64,
    project_id: CString,
    engine_version: CString,
}

impl PreparedInfo {
    fn new(info: InitInfo) -> anyhow::Result<Self> {
        Self::path_context(&info.application_data_path, "ngx application data path")?;
        Self::path_context(&info.runtime_path, "ngx runtime path")?;

        std::fs::create_dir_all(&info.application_data_path).with_context(|| {
            format!(
                "creating ngx application data directory {}",
                info.application_data_path.display()
            )
        })?;

        anyhow::ensure!(
            info.runtime_path.join(DLSS_RUNTIME_FILE).is_file(),
            "dlssd runtime is missing {}",
            info.runtime_path.join(DLSS_RUNTIME_FILE).display()
        );

        let (application_id, project_id, engine_version) = match info.identity.kind {
            IdentityKind::ApplicationId(application_id) => (
                application_id,
                CString::new("").expect("empty strings contain no null bytes"),
                CString::new("").expect("empty strings contain no null bytes"),
            ),
            IdentityKind::Project {
                project_id,
                engine_version,
            } => (
                0,
                CString::new(project_id).context("converting nvidia project id")?,
                CString::new(engine_version).context("converting nvidia engine version")?,
            ),
        };

        Ok(Self {
            application_data_path: Self::path_cstring(&info.application_data_path)?,
            runtime_path: Self::path_cstring(&info.runtime_path)?,
            application_id,
            project_id,
            engine_version,
        })
    }

    fn identity(&self) -> NativeIdentity {
        NativeIdentity {
            application_id: self.application_id,
            project_id: self.project_id.as_ptr(),
            engine_version: self.engine_version.as_ptr(),
        }
    }

    fn path_context(path: &Path, purpose: &str) -> anyhow::Result<()> {
        anyhow::ensure!(path.is_absolute(), "{purpose} must be an absolute path");

        path.to_str()
            .with_context(|| format!("{purpose} is not valid utf-8: {}", path.display()))?;

        Ok(())
    }

    fn path_cstring(path: &Path) -> anyhow::Result<CString> {
        CString::new(path.as_os_str().as_bytes())
            .with_context(|| format!("path contains a null byte: {}", path.display()))
    }
}

fn check_result(result: i32) -> anyhow::Result<()> {
    anyhow::ensure!(result == 0, "{}", result_name(result));

    Ok(())
}

fn result_name(result: i32) -> String {
    let name = unsafe { nvidia_dlss_ngx_result_name(result) };

    if name.is_null() {
        return format!("ngx result {result:#x}");
    }

    format!(
        "{} ({:#x})",
        unsafe { CStr::from_ptr(name) }.to_string_lossy(),
        result.cast_unsigned()
    )
}

#[cfg(test)]
mod tests {
    use {super::*, std::mem::size_of};

    #[test]
    fn native_extension_output_matches_cpp() {
        assert_eq!(size_of::<NativeExtensions>(), 4 + 32 * 256);
    }
}
