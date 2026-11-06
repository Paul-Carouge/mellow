//! FFmpeg's C API: `rusty_ffmpeg`'s bindings, plus the hardware-context
//! structure its header list leaves out (`libavutil/hwcontext_drm.h`, stable
//! since FFmpeg 4). The same module as `vesper-encode`'s, without its VAAPI
//! device context.

#![allow(
    non_camel_case_types,
    non_snake_case,
    reason = "the names of FFmpeg's C structures"
)]

use std::ffi::{CStr, CString, NulError, c_int};
use std::os::unix::ffi::OsStringExt;

pub(crate) use rusty_ffmpeg::ffi::*;

/// Sets a colour option (`colorspace` or `range`) of a `buffer` source, the
/// options behind `AVBufferSrcParameters.color_space` and `.color_range`
/// (`FFmpeg` 7.1). Older `FFmpeg` (6.1 on Ubuntu 24.04) has neither: the source
/// then keeps the frames' own colour properties, and this returns 0.
/// Otherwise returns `FFmpeg`'s status.
///
/// # Safety
/// `buffer_source` is a live `buffer` filter context, not yet initialised.
pub(crate) unsafe fn set_buffer_source_colour_option(
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
pub(crate) fn render_node_c_string() -> Result<CString, NulError> {
    CString::new(
        vesper_core::discover_render_node()
            .into_os_string()
            .into_vec(),
    )
}

/// `AV_DRM_MAX_PLANES`.
pub(crate) const AV_DRM_MAX_PLANES: usize = 4;

/// `AVDRMObjectDescriptor`: one DMA-BUF.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct AVDRMObjectDescriptor {
    pub(crate) fd: c_int,
    pub(crate) size: usize,
    pub(crate) format_modifier: u64,
}

/// `AVDRMPlaneDescriptor`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct AVDRMPlaneDescriptor {
    pub(crate) object_index: c_int,
    pub(crate) offset: isize,
    pub(crate) pitch: isize,
}

/// `AVDRMLayerDescriptor`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct AVDRMLayerDescriptor {
    pub(crate) format: u32,
    pub(crate) nb_planes: c_int,
    pub(crate) planes: [AVDRMPlaneDescriptor; AV_DRM_MAX_PLANES],
}

/// `AVDRMFrameDescriptor`: what an `AV_PIX_FMT_DRM_PRIME` frame's `data[0]`
/// points to.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct AVDRMFrameDescriptor {
    pub(crate) nb_objects: c_int,
    pub(crate) objects: [AVDRMObjectDescriptor; AV_DRM_MAX_PLANES],
    pub(crate) nb_layers: c_int,
    pub(crate) layers: [AVDRMLayerDescriptor; AV_DRM_MAX_PLANES],
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
    }
}
