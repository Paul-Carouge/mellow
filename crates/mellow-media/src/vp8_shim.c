/*
 * A narrow C face over libvpx's VP8 encoder and decoder for `src/vp8.rs`. libvpx's structures
 * (`vpx_codec_enc_cfg_t`, `vpx_image_t`, packets) never cross into Rust: Rust only sees opaque
 * pointers and plain integers, so no hand-written struct layout can drift from the headers.
 */

#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "vpx/vp8cx.h"
#include "vpx/vp8dx.h"
#include "vpx/vpx_decoder.h"
#include "vpx/vpx_encoder.h"

/* Results, mirrored by `ShimStatus` in `src/vp8.rs`. */
enum {
  MELLOW_VP8_OK = 0,
  MELLOW_VP8_CODEC_ERROR = -1,
  MELLOW_VP8_OUTPUT_TOO_SMALL = -2,
  MELLOW_VP8_OUT_OF_MEMORY = -3,
  MELLOW_VP8_NO_FRAME = -4,
};

/* RTP's video clock: presentation times are given in it. */
#define MELLOW_VP8_TIMEBASE_DENOMINATOR 90000

typedef struct {
  vpx_codec_ctx_t codec;
  vpx_codec_enc_cfg_t config;
} mellow_vp8_encoder;

typedef struct {
  vpx_codec_ctx_t codec;
  /* The last decoded picture, valid until the next decode. */
  vpx_image_t *picture;
} mellow_vp8_decoder;

/*
 * Chrome's real-time VP8 settings (`LibvpxVp8Encoder`): one pass, constant bit rate, no
 * look-ahead, error-resilient, frames dropped under pressure rather than late, intra frames capped
 * so a keyframe fits the server's per-layer byte budget (ADR-0035).
 */
int mellow_vp8_encoder_new(mellow_vp8_encoder **out, unsigned int width, unsigned int height,
                           unsigned int bitrate_kilobits, unsigned int frames_per_second,
                           unsigned int threads, int cpu_used, int is_screen) {
  mellow_vp8_encoder *encoder = calloc(1, sizeof(*encoder));
  if (encoder == NULL) return MELLOW_VP8_OUT_OF_MEMORY;
  if (vpx_codec_enc_config_default(vpx_codec_vp8_cx(), &encoder->config, 0) != VPX_CODEC_OK) {
    free(encoder);
    return MELLOW_VP8_CODEC_ERROR;
  }
  vpx_codec_enc_cfg_t *config = &encoder->config;
  config->g_w = width;
  config->g_h = height;
  config->g_threads = threads;
  config->g_timebase.num = 1;
  config->g_timebase.den = MELLOW_VP8_TIMEBASE_DENOMINATOR;
  config->g_lag_in_frames = 0;
  config->g_pass = VPX_RC_ONE_PASS;
  config->g_error_resilient = VPX_ERROR_RESILIENT_DEFAULT;
  config->rc_end_usage = VPX_CBR;
  config->rc_target_bitrate = bitrate_kilobits;
  config->rc_min_quantizer = 2;
  config->rc_max_quantizer = is_screen ? 52 : 56;
  config->rc_undershoot_pct = 100;
  config->rc_overshoot_pct = 15;
  config->rc_buf_initial_sz = 500;
  config->rc_buf_optimal_sz = 600;
  config->rc_buf_sz = 1000;
  /* A screen keeps its sharpness and drops frames; a camera keeps its motion. */
  config->rc_dropframe_thresh = is_screen ? 0 : 30;
  config->rc_resize_allowed = 0;
  config->kf_mode = VPX_KF_AUTO;
  config->kf_max_dist = 3000;
  if (vpx_codec_enc_init(&encoder->codec, vpx_codec_vp8_cx(), config, 0) != VPX_CODEC_OK) {
    free(encoder);
    return MELLOW_VP8_CODEC_ERROR;
  }
  /* Chrome's cap: half the optimal buffer, in percent of an average frame. */
  unsigned int max_intra_percent =
      config->rc_buf_optimal_sz / 2 * (frames_per_second > 0 ? frames_per_second : 30) / 10;
  if (max_intra_percent < 300) max_intra_percent = 300;
  int token_partitions = width * height >= 1280 * 720 ? VP8_TWO_TOKENPARTITION : VP8_ONE_TOKENPARTITION;
  if (vpx_codec_control(&encoder->codec, VP8E_SET_CPUUSED, cpu_used) != VPX_CODEC_OK ||
      vpx_codec_control(&encoder->codec, VP8E_SET_NOISE_SENSITIVITY, 0u) != VPX_CODEC_OK ||
      vpx_codec_control(&encoder->codec, VP8E_SET_STATIC_THRESHOLD, is_screen ? 100u : 1u) !=
          VPX_CODEC_OK ||
      vpx_codec_control(&encoder->codec, VP8E_SET_TOKEN_PARTITIONS, token_partitions) !=
          VPX_CODEC_OK ||
      vpx_codec_control(&encoder->codec, VP8E_SET_MAX_INTRA_BITRATE_PCT, max_intra_percent) !=
          VPX_CODEC_OK ||
      vpx_codec_control(&encoder->codec, VP8E_SET_SCREEN_CONTENT_MODE, is_screen ? 1u : 0u) !=
          VPX_CODEC_OK) {
    vpx_codec_destroy(&encoder->codec);
    free(encoder);
    return MELLOW_VP8_CODEC_ERROR;
  }
  *out = encoder;
  return MELLOW_VP8_OK;
}

int mellow_vp8_encoder_set_bitrate(mellow_vp8_encoder *encoder, unsigned int bitrate_kilobits) {
  encoder->config.rc_target_bitrate = bitrate_kilobits;
  return vpx_codec_enc_config_set(&encoder->codec, &encoder->config) == VPX_CODEC_OK
             ? MELLOW_VP8_OK
             : MELLOW_VP8_CODEC_ERROR;
}

/*
 * Encodes one I420 picture of the encoder's size into `output`. `*written` is 0 when the rate
 * control dropped the frame.
 */
int mellow_vp8_encode(mellow_vp8_encoder *encoder, const uint8_t *luma, int luma_stride,
                      const uint8_t *blue_chroma, const uint8_t *red_chroma, int chroma_stride,
                      int64_t presentation_time, unsigned long duration, int force_keyframe,
                      uint8_t *output, size_t output_capacity, size_t *written,
                      int *is_keyframe) {
  vpx_image_t picture;
  /* Wrapping only describes the planes; nothing is copied or freed by libvpx. */
  if (vpx_img_wrap(&picture, VPX_IMG_FMT_I420, encoder->config.g_w, encoder->config.g_h, 1,
                   (unsigned char *)luma) == NULL) {
    return MELLOW_VP8_CODEC_ERROR;
  }
  picture.planes[VPX_PLANE_Y] = (unsigned char *)luma;
  picture.planes[VPX_PLANE_U] = (unsigned char *)blue_chroma;
  picture.planes[VPX_PLANE_V] = (unsigned char *)red_chroma;
  picture.stride[VPX_PLANE_Y] = luma_stride;
  picture.stride[VPX_PLANE_U] = chroma_stride;
  picture.stride[VPX_PLANE_V] = chroma_stride;
  vpx_enc_frame_flags_t flags = force_keyframe ? VPX_EFLAG_FORCE_KF : 0;
  if (vpx_codec_encode(&encoder->codec, &picture, presentation_time, duration, flags,
                       VPX_DL_REALTIME) != VPX_CODEC_OK) {
    return MELLOW_VP8_CODEC_ERROR;
  }
  *written = 0;
  *is_keyframe = 0;
  vpx_codec_iter_t iterator = NULL;
  const vpx_codec_cx_pkt_t *packet;
  while ((packet = vpx_codec_get_cx_data(&encoder->codec, &iterator)) != NULL) {
    if (packet->kind != VPX_CODEC_CX_FRAME_PKT) continue;
    size_t size = packet->data.frame.sz;
    if (size > output_capacity - *written) return MELLOW_VP8_OUTPUT_TOO_SMALL;
    memcpy(output + *written, packet->data.frame.buf, size);
    *written += size;
    if (packet->data.frame.flags & VPX_FRAME_IS_KEY) *is_keyframe = 1;
  }
  return MELLOW_VP8_OK;
}

void mellow_vp8_encoder_free(mellow_vp8_encoder *encoder) {
  if (encoder == NULL) return;
  vpx_codec_destroy(&encoder->codec);
  free(encoder);
}

int mellow_vp8_decoder_new(mellow_vp8_decoder **out, unsigned int threads) {
  mellow_vp8_decoder *decoder = calloc(1, sizeof(*decoder));
  if (decoder == NULL) return MELLOW_VP8_OUT_OF_MEMORY;
  vpx_codec_dec_cfg_t config = {threads, 0, 0};
  if (vpx_codec_dec_init(&decoder->codec, vpx_codec_vp8_dx(), &config, 0) != VPX_CODEC_OK) {
    free(decoder);
    return MELLOW_VP8_CODEC_ERROR;
  }
  *out = decoder;
  return MELLOW_VP8_OK;
}

/*
 * Decodes one frame; on success the picture's planes are described through the out parameters
 * and stay valid until the next call. `MELLOW_VP8_NO_FRAME` when the frame shows nothing.
 */
int mellow_vp8_decode(mellow_vp8_decoder *decoder, const uint8_t *frame, size_t frame_length,
                      unsigned int *width, unsigned int *height, const uint8_t **luma,
                      int *luma_stride, const uint8_t **blue_chroma, const uint8_t **red_chroma,
                      int *chroma_stride) {
  decoder->picture = NULL;
  if (frame_length > UINT32_MAX) return MELLOW_VP8_CODEC_ERROR;
  if (vpx_codec_decode(&decoder->codec, frame, (unsigned int)frame_length, NULL, 0) !=
      VPX_CODEC_OK) {
    return MELLOW_VP8_CODEC_ERROR;
  }
  vpx_codec_iter_t iterator = NULL;
  vpx_image_t *picture = NULL;
  vpx_image_t *next;
  /* VP8 shows at most one picture per frame; keep the last should that ever change. */
  while ((next = vpx_codec_get_frame(&decoder->codec, &iterator)) != NULL) picture = next;
  if (picture == NULL) return MELLOW_VP8_NO_FRAME;
  if (picture->fmt != VPX_IMG_FMT_I420 ||
      picture->stride[VPX_PLANE_U] != picture->stride[VPX_PLANE_V]) {
    return MELLOW_VP8_CODEC_ERROR;
  }
  decoder->picture = picture;
  *width = picture->d_w;
  *height = picture->d_h;
  *luma = picture->planes[VPX_PLANE_Y];
  *blue_chroma = picture->planes[VPX_PLANE_U];
  *red_chroma = picture->planes[VPX_PLANE_V];
  *luma_stride = picture->stride[VPX_PLANE_Y];
  *chroma_stride = picture->stride[VPX_PLANE_U];
  return MELLOW_VP8_OK;
}

void mellow_vp8_decoder_free(mellow_vp8_decoder *decoder) {
  if (decoder == NULL) return;
  vpx_codec_destroy(&decoder->codec);
  free(decoder);
}
