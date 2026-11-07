//! `FFmpeg`'s C API: `rusty_ffmpeg`'s bindings, plus the two hardware-context
//! structures its header list leaves out (`libavutil/hwcontext_drm.h` and
//! `libavutil/hwcontext_vaapi.h`). Both layouts are stable since `FFmpeg` 4.

#![allow(
    non_camel_case_types,
    non_snake_case,
    reason = "the names of FFmpeg's C structures"
)]

use std::ffi::{CStr, CString, NulError, c_int, c_uint, c_void};
use std::os::unix::ffi::OsStringExt;

pub use rusty_ffmpeg::ffi::*;

/// Sets a colour option (`colorspace` or `range`) of a `buffer` source, the
/// options behind `AVBufferSrcParameters.color_space` and `.color_range`
/// (`FFmpeg` 7.1). Older `FFmpeg` (6.1 on Ubuntu 24.04) has neither: the source
/// then keeps the frames' own colour properties, and this returns 0.
/// Otherwise returns `FFmpeg`'s status.
///
/// # Safety
/// `buffer_source` is a live `buffer` filter context, not yet initialised.
pub unsafe fn set_buffer_source_colour_option(
    buffer_source: *mut AVFilterContext,
    option: &CStr,
    value: i64,
) -> c_int {
    // SAFETY: the caller hands a live filter context; `option` is a C string.
    let status = unsafe {
        av_opt_set_int(
            buffer_source.cast(),
            option.as_ptr(),
            value,
            AV_OPT_SEARCH_CHILDREN as c_int,
        )
    };
    if status == AVERROR_OPTION_NOT_FOUND {
        0
    } else {
        status
    }
}

/// [`vesper_core::discover_render_node`] as the C string
/// `av_hwdevice_ctx_create` takes.
///
/// # Errors
/// When the path holds a NUL byte (never for a `/dev/dri` entry).
pub fn render_node_c_string() -> Result<CString, NulError> {
    CString::new(
        vesper_core::discover_render_node()
            .into_os_string()
            .into_vec(),
    )
}

/// `AV_DRM_MAX_PLANES`.
pub const AV_DRM_MAX_PLANES: usize = 4;

/// `AVDRMObjectDescriptor`: one DMA-BUF.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AVDRMObjectDescriptor {
    pub fd: c_int,
    pub size: usize,
    pub format_modifier: u64,
}

/// `AVDRMPlaneDescriptor`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AVDRMPlaneDescriptor {
    pub object_index: c_int,
    pub offset: isize,
    pub pitch: isize,
}

/// `AVDRMLayerDescriptor`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AVDRMLayerDescriptor {
    pub format: u32,
    pub nb_planes: c_int,
    pub planes: [AVDRMPlaneDescriptor; AV_DRM_MAX_PLANES],
}

/// `AVDRMFrameDescriptor`: what an `AV_PIX_FMT_DRM_PRIME` frame's `data[0]`
/// points to.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AVDRMFrameDescriptor {
    pub nb_objects: c_int,
    pub objects: [AVDRMObjectDescriptor; AV_DRM_MAX_PLANES],
    pub nb_layers: c_int,
    pub layers: [AVDRMLayerDescriptor; AV_DRM_MAX_PLANES],
}

/// `AVVAAPIDeviceContext`: what a VAAPI `AVHWDeviceContext`'s `hwctx`
/// points to.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AVVAAPIDeviceContext {
    pub display: *mut c_void,
    pub driver_quirks: c_uint,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_match_ffmpeg_on_64_bit_linux() {
        assert_eq!(size_of::<AVDRMObjectDescriptor>(), 24);
        assert_eq!(size_of::<AVDRMPlaneDescriptor>(), 24);
        assert_eq!(size_of::<AVDRMLayerDescriptor>(), 104);
        assert_eq!(size_of::<AVDRMFrameDescriptor>(), 528);
        assert_eq!(std::mem::offset_of!(AVDRMFrameDescriptor, layers), 112);
        assert_eq!(size_of::<AVVAAPIDeviceContext>(), 16);
    }
}
