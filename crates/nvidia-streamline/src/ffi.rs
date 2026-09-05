use {
    crate::{DlssRrFrameConstants, DlssRrResources},
    std::ffi::{c_char, c_void},
};

#[repr(C)]
pub(crate) struct Context {
    _private: [u8; 0],
}

#[derive(Clone, Copy)]
#[repr(C)]
pub(crate) struct Extent {
    pub(crate) width: u32,
    pub(crate) height: u32,
}

unsafe extern "C" {
    pub(crate) fn nvidia_sl_init(
        interposer_path: *const u16,
        plugin_path: *const u16,
        project_id: *const c_char,
        engine_version: *const c_char,
        log_callback: unsafe extern "C" fn(i32, *const c_char),
        output: *mut *mut Context,
    ) -> i32;
    pub(crate) fn nvidia_sl_is_dlss_rr_supported(
        context: *mut Context,
        physical_device: *mut c_void,
    ) -> i32;
    pub(crate) fn nvidia_sl_dlss_rr_optimal_settings(
        context: *mut Context,
        output_width: u32,
        output_height: u32,
        quality: u32,
        output: *mut Extent,
    ) -> i32;
    pub(crate) fn nvidia_sl_evaluate_dlss_rr(
        context: *mut Context,
        command_buffer: *mut c_void,
        quality: u32,
        frame: *const DlssRrFrameConstants,
        resources: *const DlssRrResources,
    ) -> i32;
    pub(crate) fn nvidia_sl_shutdown(context: *mut Context) -> i32;
    pub(crate) fn nvidia_sl_result_name(result: i32) -> *const c_char;
}
