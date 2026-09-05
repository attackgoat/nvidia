#![cfg_attr(test, allow(dead_code))]

use {
    crate::{OpacityMicromapDescriptor, OpacityMicromapUsage, Statistics},
    std::ffi::{c_char, c_void},
};

unsafe extern "C" {
    pub(crate) fn nvidia_omm_create(library_path: *const c_char, output: *mut *mut Context) -> i32;
    pub(crate) fn nvidia_omm_destroy(context: *mut Context) -> i32;
    pub(crate) fn nvidia_omm_create_texture(
        context: *mut Context,
        mips: *const TextureMip,
        mip_count: u32,
        output: *mut *mut Texture,
    ) -> i32;
    pub(crate) fn nvidia_omm_destroy_texture(context: *mut Context, texture: *mut Texture) -> i32;
    pub(crate) fn nvidia_omm_bake(
        context: *mut Context,
        input: *const BakeInput,
        output: *mut BakeResult,
        result: *mut *mut c_void,
    ) -> i32;
    pub(crate) fn nvidia_omm_destroy_bake_result(result: *mut c_void) -> i32;
    pub(crate) fn nvidia_omm_result_name(result: i32) -> *const c_char;
}

#[derive(Clone, Copy)]
#[repr(C)]
pub(crate) struct BakeInput {
    pub(crate) texture: *mut Texture,
    pub(crate) texture_coordinates: *const [f32; 2],
    pub(crate) texture_coordinate_count: u32,
    pub(crate) indices: *const u32,
    pub(crate) index_count: u32,
    pub(crate) dynamic_subdivision_scale: f32,
    pub(crate) rejection_threshold: f32,
    pub(crate) max_array_data_size: u32,
    pub(crate) max_workload_size: u64,
    pub(crate) max_subdivision_level: u8,
    pub(crate) internal_threads: u8,
    pub(crate) validation: u8,
    pub(crate) reserved: [u8; 5],
}

#[derive(Clone, Copy)]
#[repr(C)]
pub(crate) struct BakeResult {
    pub(crate) array_data: *const u8,
    pub(crate) array_data_size: u32,
    pub(crate) descriptors: *const OpacityMicromapDescriptor,
    pub(crate) descriptor_count: u32,
    pub(crate) descriptor_usage: *const OpacityMicromapUsage,
    pub(crate) descriptor_usage_count: u32,
    pub(crate) indices: *const i32,
    pub(crate) index_count: u32,
    pub(crate) index_usage: *const OpacityMicromapUsage,
    pub(crate) index_usage_count: u32,
    pub(crate) statistics: Statistics,
}

#[repr(C)]
pub(crate) struct Context {
    _private: [u8; 0],
}

#[repr(C)]
pub(crate) struct Texture {
    _private: [u8; 0],
}

#[derive(Clone, Copy)]
#[repr(C)]
pub(crate) struct TextureMip {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) row_pitch: u32,
    pub(crate) reserved: u32,
    pub(crate) data: *const u8,
}

#[cfg(test)]
mod test {
    use {super::*, std::mem::size_of};

    #[test]
    fn native_ffi_layouts_are_stable() {
        assert_eq!(size_of::<BakeInput>(), 64);
        assert_eq!(size_of::<BakeResult>(), 136);
        assert_eq!(size_of::<TextureMip>(), 24);
    }
}
