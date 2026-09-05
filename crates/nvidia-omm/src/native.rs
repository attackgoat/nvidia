use {
    crate::{AlphaMip, BakeInput, BakeOutput, ffi},
    anyhow::{Context as _, Result, ensure},
    std::{
        ffi::{CStr, CString, c_void},
        path::Path,
        ptr::NonNull,
        sync::Mutex,
    },
};

static API_LOCK: Mutex<()> = Mutex::new(());

fn ensure_result(operation: &str, result: i32) -> Result<()> {
    ensure!(result == 0, "{operation}: {}", result_name(result));

    Ok(())
}

fn result_name(result: i32) -> String {
    let name = unsafe { ffi::nvidia_omm_result_name(result) };

    if name.is_null() {
        return format!("unknown result {result}");
    }

    unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned()
}

fn lock_api() -> std::sync::MutexGuard<'static, ()> {
    API_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub struct AlphaTexture<'a> {
    baker: &'a Baker,
    raw: NonNull<ffi::Texture>,
    size: [u32; 2],
}

impl Drop for AlphaTexture<'_> {
    fn drop(&mut self) {
        let _guard = lock_api();

        let result =
            unsafe { ffi::nvidia_omm_destroy_texture(self.baker.raw.as_ptr(), self.raw.as_ptr()) };

        debug_assert_eq!(
            result,
            0,
            "destroying nvidia omm texture failed: {}",
            result_name(result)
        );
    }
}

pub struct Baker {
    raw: NonNull<ffi::Context>,
}

impl Baker {
    /// Creates a CPU baker from an absolute backend library path.
    ///
    /// # Errors
    /// Returns an error for an invalid path, backend loading failure, or SDK creation failure.
    pub fn new(library_path: &Path) -> Result<Self> {
        ensure!(
            library_path.is_absolute(),
            "nvidia omm library path must be absolute"
        );

        let library_path = CString::new(
            library_path
                .to_str()
                .context("nvidia omm library path is not valid utf-8")?,
        )
        .context("nvidia omm library path contains a null byte")?;

        let _guard = lock_api();
        let mut raw = std::ptr::null_mut();

        let result = unsafe { ffi::nvidia_omm_create(library_path.as_ptr(), &raw mut raw) };

        ensure_result("creating nvidia omm cpu baker", result)?;

        Ok(Self {
            raw: NonNull::new(raw).context("nvidia omm returned a null baker")?,
        })
    }

    /// Copies alpha mips into an SDK texture owned by this baker.
    ///
    /// # Errors
    /// Returns an error for invalid mip dimensions, pitches, or data lengths, too many
    /// mips, or an SDK texture creation failure.
    pub fn create_texture<'a>(&'a self, mips: &[AlphaMip<'_>]) -> Result<AlphaTexture<'a>> {
        AlphaMip::validate_all(mips)?;

        let native_mips = mips
            .iter()
            .map(|mip| ffi::TextureMip {
                width: mip.width,
                height: mip.height,
                row_pitch: mip.row_pitch,
                reserved: 0,
                data: mip.data.as_ptr(),
            })
            .collect::<Vec<_>>();
        let mip_count = u32::try_from(native_mips.len()).context("too many nvidia omm mips")?;
        let _guard = lock_api();
        let mut raw = std::ptr::null_mut();

        let result = unsafe {
            ffi::nvidia_omm_create_texture(
                self.raw.as_ptr(),
                native_mips.as_ptr(),
                mip_count,
                &raw mut raw,
            )
        };

        ensure_result("creating nvidia omm alpha texture", result)?;

        Ok(AlphaTexture {
            baker: self,
            raw: NonNull::new(raw).context("nvidia omm returned a null texture")?,
            size: [mips[0].width, mips[0].height],
        })
    }

    /// Bakes opacity micromaps and copies the results into owned storage.
    ///
    /// # Errors
    /// Returns an error if the texture belongs to another baker, the geometry or
    /// configuration is invalid, input counts exceed SDK limits, or the SDK fails
    /// or returns inconsistent output.
    pub fn bake(&self, texture: &AlphaTexture<'_>, input: BakeInput<'_>) -> Result<BakeOutput> {
        ensure!(
            std::ptr::eq(self, texture.baker),
            "nvidia omm texture belongs to another baker"
        );

        input.validate(texture.size)?;

        let native_input = ffi::BakeInput {
            texture: texture.raw.as_ptr(),
            texture_coordinates: input.texture_coordinates.as_ptr(),
            texture_coordinate_count: u32::try_from(input.texture_coordinates.len())?,
            indices: input.indices.as_ptr(),
            index_count: u32::try_from(input.indices.len())?,
            dynamic_subdivision_scale: input.config.dynamic_subdivision_scale,
            rejection_threshold: input.config.rejection_threshold,
            max_array_data_size: input.config.max_array_data_size,
            max_workload_size: input.config.max_workload_size,
            max_subdivision_level: input.config.max_subdivision_level,
            internal_threads: input.config.internal_threads.into(),
            validation: input.config.validation.into(),
            reserved: [0; 5],
        };

        let _guard = lock_api();

        let mut native_output = unsafe { std::mem::zeroed::<ffi::BakeResult>() };

        let mut result_handle = std::ptr::null_mut();

        let result = unsafe {
            ffi::nvidia_omm_bake(
                self.raw.as_ptr(),
                &raw const native_input,
                &raw mut native_output,
                &raw mut result_handle,
            )
        };

        ensure_result("baking nvidia opacity micromaps", result)?;

        let result_handle =
            NonNull::new(result_handle).context("nvidia omm returned a null bake result")?;
        let result_guard = BakeResultGuard(result_handle);

        ensure!(
            native_output.index_count as usize == input.indices.len() / 3,
            "nvidia omm output index count does not match the input triangle count"
        );

        let output = unsafe { BakeOutput::from_native(native_output)? };

        drop(result_guard);

        Ok(output)
    }
}

impl Drop for Baker {
    fn drop(&mut self) {
        let _guard = lock_api();

        let result = unsafe { ffi::nvidia_omm_destroy(self.raw.as_ptr()) };

        debug_assert_eq!(
            result,
            0,
            "destroying nvidia omm baker failed: {}",
            result_name(result)
        );
    }
}

unsafe impl Send for Baker {}

unsafe impl Sync for Baker {}

struct BakeResultGuard(NonNull<c_void>);

impl Drop for BakeResultGuard {
    fn drop(&mut self) {
        let result = unsafe { ffi::nvidia_omm_destroy_bake_result(self.0.as_ptr()) };

        debug_assert_eq!(
            result,
            0,
            "destroying nvidia omm result failed: {}",
            result_name(result)
        );
    }
}
