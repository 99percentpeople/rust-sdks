// Copyright 2026 LiveKit, Inc.
// SPDX-License-Identifier: Apache-2.0
#include "livekit/vp9_i444_encoder.h"

#if defined(WEBRTC_WIN)
#include <algorithm>
#include <cmath>

#include "api/video/i444_buffer.h"
#include "modules/video_coding/include/video_codec_interface.h"
#include "modules/video_coding/include/video_error_codes.h"
#include "third_party/libvpx/source/libvpx/vpx/vp8cx.h"
#include "third_party/libvpx/source/libvpx/vpx/vpx_encoder.h"

namespace livekit_ffi {
namespace {
class Vp9I444Encoder final : public webrtc::VideoEncoder {
 public:
  ~Vp9I444Encoder() override { Release(); }
  int32_t InitEncode(const webrtc::VideoCodec* codec,
                     const Settings& settings) override {
    Release();
    if (!codec || !codec->width || !codec->height || !codec->maxFramerate ||
        codec->numberOfSimulcastStreams > 1 ||
        codec->VP9().numberOfSpatialLayers > 1 ||
        codec->VP9().numberOfTemporalLayers > 1)
      return WEBRTC_VIDEO_CODEC_ERR_PARAMETER;
    if (vpx_codec_enc_config_default(vpx_codec_vp9_cx(), &config_, 0) !=
        VPX_CODEC_OK)
      return WEBRTC_VIDEO_CODEC_ERROR;
    config_.g_profile = 1;
    config_.g_bit_depth = VPX_BITS_8;
    config_.g_input_bit_depth = 8;
    config_.g_w = codec->width;
    config_.g_h = codec->height;
    config_.g_threads = std::clamp(settings.number_of_cores, 1, 8);
    config_.g_timebase = {1, 90000};
    config_.g_lag_in_frames = 0;
    config_.g_error_resilient = VPX_ERROR_RESILIENT_DEFAULT;
    config_.rc_end_usage = VPX_CBR;
    config_.rc_target_bitrate = std::max(1u, codec->startBitrate);
    config_.rc_min_quantizer = 2;
    config_.rc_max_quantizer = 52;
    config_.rc_dropframe_thresh =
        0;  // WebRTC drops raw input before this encoder.
    config_.rc_resize_allowed = 0;
    config_.rc_buf_initial_sz = 300;
    config_.rc_buf_optimal_sz = 300;
    config_.rc_buf_sz = 600;
    config_.kf_mode = VPX_KF_DISABLED;
    config_.kf_max_dist = 0;
    fps_ = codec->maxFramerate;
    if (vpx_codec_enc_init(&encoder_, vpx_codec_vp9_cx(), &config_, 0) !=
        VPX_CODEC_OK) {
      Release();
      return WEBRTC_VIDEO_CODEC_ERROR;
    }
    initialized_ = true;
    if (vpx_codec_control(&encoder_, VP8E_SET_CPUUSED, 7) != VPX_CODEC_OK ||
        vpx_codec_control(&encoder_, VP8E_SET_ENABLEAUTOALTREF, 0u) !=
            VPX_CODEC_OK ||
        vpx_codec_control(&encoder_, VP9E_SET_ROW_MT, 1u) != VPX_CODEC_OK) {
      Release();
      return WEBRTC_VIDEO_CODEC_ERROR;
    }
    // No lookahead or asynchronous work: the encoder owns no threads/callback
    // captures outside libvpx, which codec_destroy joins before releasing
    // memory.
    vpx_codec_control(&encoder_, VP9E_SET_TILE_COLUMNS,
                      config_.g_w >= 1280 ? 2 : 0);
    vpx_codec_control(&encoder_, VP9E_SET_AQ_MODE, 0u);
    return WEBRTC_VIDEO_CODEC_OK;
  }
  int32_t RegisterEncodeCompleteCallback(
      webrtc::EncodedImageCallback* callback) override {
    callback_ = callback;
    return WEBRTC_VIDEO_CODEC_OK;
  }
  int32_t Release() override {
    if (encoder_.priv)
      vpx_codec_destroy(&encoder_);
    initialized_ = false;
    encoder_ = {};
    paused_ = false;
    keyframe_ = true;
    timestamp_ = 0;
    color_ = -1;
    range_ = -1;
    return WEBRTC_VIDEO_CODEC_OK;
  }
  void SetFecControllerOverride(webrtc::FecControllerOverride*) override {}
  void SetRates(const RateControlParameters& rates) override {
    if (!initialized_ || !std::isfinite(rates.framerate_fps) ||
        rates.framerate_fps < 1)
      return;
    const auto bitrate = rates.bitrate.get_sum_bps();
    if (paused_ && bitrate)
      keyframe_ = true;
    paused_ = bitrate == 0;
    if (paused_)
      return;
    fps_ = rates.framerate_fps;
    config_.rc_target_bitrate = std::max(1u, bitrate / 1000);
    if (vpx_codec_enc_config_set(&encoder_, &config_) != VPX_CODEC_OK) {
      Release();  // A failed rate change must not continue sending at the old
                  // rate.
    }
  }
  int32_t Encode(const webrtc::VideoFrame& frame,
                 const std::vector<webrtc::VideoFrameType>* types) override {
    if (!initialized_ || !callback_)
      return WEBRTC_VIDEO_CODEC_UNINITIALIZED;
    if (paused_) {
      callback_->OnFrameDropped(frame.rtp_timestamp(), 0, true);
      return WEBRTC_VIDEO_CODEC_OK;
    }
    // WebRTC may emit an I420 startup/placeholder frame before the first
    // capture. GetI444 DCHECKs on other buffer types, so inspect the type
    // first.
    if (frame.video_frame_buffer()->type() !=
        webrtc::VideoFrameBuffer::Type::kI444) {
      callback_->OnFrameDropped(frame.rtp_timestamp(), 0, true);
      return WEBRTC_VIDEO_CODEC_OK;
    }
    const auto* pixels = frame.video_frame_buffer()->GetI444();
    if (!pixels || pixels->width() != static_cast<int>(config_.g_w) ||
        pixels->height() != static_cast<int>(config_.g_h))
      return WEBRTC_VIDEO_CODEC_ERR_PARAMETER;
    const auto& space = frame.color_space();
    const int color =
        space && space->matrix() == webrtc::ColorSpace::MatrixID::kRGB
            ? VPX_CS_SRGB
        : space && space->matrix() == webrtc::ColorSpace::MatrixID::kBT709
            ? VPX_CS_BT_709
            : VPX_CS_SMPTE_170;
    const int range =
        space && space->range() == webrtc::ColorSpace::RangeID::kFull;
    if (color == VPX_CS_SRGB && !range)
      return WEBRTC_VIDEO_CODEC_ERR_PARAMETER;
    if (color != color_ || range != range_) {
      if (vpx_codec_control(&encoder_, VP9E_SET_COLOR_SPACE, color) !=
              VPX_CODEC_OK ||
          vpx_codec_control(&encoder_, VP9E_SET_COLOR_RANGE, range) !=
              VPX_CODEC_OK)
        return WEBRTC_VIDEO_CODEC_ERROR;
      color_ = color;
      range_ = range;
      keyframe_ = true;
    }
    vpx_image_t image = {};
    image.fmt = VPX_IMG_FMT_I444;
    image.w = image.d_w = config_.g_w;
    image.h = image.d_h = config_.g_h;
    image.bit_depth = 8;
    image.cs = static_cast<vpx_color_space_t>(color);
    image.range = range ? VPX_CR_FULL_RANGE : VPX_CR_STUDIO_RANGE;
    image.planes[0] = const_cast<uint8_t*>(pixels->DataY());
    image.planes[1] = const_cast<uint8_t*>(pixels->DataU());
    image.planes[2] = const_cast<uint8_t*>(pixels->DataV());
    image.stride[0] = pixels->StrideY();
    image.stride[1] = pixels->StrideU();
    image.stride[2] = pixels->StrideV();
    const bool requested_key =
        types &&
        std::find(types->begin(), types->end(),
                  webrtc::VideoFrameType::kVideoFrameKey) != types->end();
    // LAST-only references match the single-layer RTP dependency description.
    vpx_enc_frame_flags_t flags = VP8_EFLAG_NO_REF_GF | VP8_EFLAG_NO_REF_ARF |
                                  VP8_EFLAG_NO_UPD_GF | VP8_EFLAG_NO_UPD_ARF;
    if (keyframe_ || requested_key)
      flags |= VPX_EFLAG_FORCE_KF;
    const auto duration =
        static_cast<unsigned long>(std::max(1.0, 90000.0 / fps_));
    if (vpx_codec_encode(&encoder_, &image, timestamp_, duration, flags,
                         VPX_DL_REALTIME) != VPX_CODEC_OK)
      return WEBRTC_VIDEO_CODEC_ERROR;
    timestamp_ += duration;
    vpx_codec_iter_t iterator = nullptr;
    bool delivered = false;
    while (const auto* packet = vpx_codec_get_cx_data(&encoder_, &iterator)) {
      if (packet->kind != VPX_CODEC_CX_FRAME_PKT)
        continue;
      const bool key = packet->data.frame.flags & VPX_FRAME_IS_KEY;
      webrtc::EncodedImage encoded;
      encoded.SetEncodedData(webrtc::EncodedImageBuffer::Create(
          static_cast<const uint8_t*>(packet->data.frame.buf),
          packet->data.frame.sz));
      encoded._encodedWidth = config_.g_w;
      encoded._encodedHeight = config_.g_h;
      encoded.SetRtpTimestamp(frame.rtp_timestamp());
      encoded.SetSimulcastIndex(0);
      encoded.capture_time_ms_ = frame.render_time_ms();
      encoded.ntp_time_ms_ = frame.ntp_time_ms();
      encoded.rotation_ = frame.rotation();
      encoded._frameType = key ? webrtc::VideoFrameType::kVideoFrameKey
                               : webrtc::VideoFrameType::kVideoFrameDelta;
      encoded.SetColorSpace(space);
      vpx_codec_control(&encoder_, VP8E_GET_LAST_QUANTIZER, &encoded.qp_);
      webrtc::CodecSpecificInfo info;
      info.codecType = webrtc::kVideoCodecVP9;
      info.codecSpecific = {};
      info.end_of_picture = true;
      auto& vp9 = info.codecSpecific.VP9;
      vp9.first_frame_in_picture = true;
      vp9.inter_pic_predicted = !key;
      vp9.flexible_mode = false;
      vp9.ss_data_available = key;
      vp9.temporal_idx = 0;
      vp9.temporal_up_switch = true;
      vp9.num_spatial_layers = 1;
      vp9.spatial_layer_resolution_present = key;
      vp9.width[0] = config_.g_w;
      vp9.height[0] = config_.g_h;
      vp9.gof.SetGofInfoVP9(webrtc::kTemporalStructureMode1);
      vp9.num_ref_pics = key ? 0 : 1;
      vp9.p_diff[0] = 1;
      delivered = true;
      keyframe_ = false;
      if (callback_->OnEncodedImage(encoded, &info).error !=
          webrtc::EncodedImageCallback::Result::OK) {
        keyframe_ = true;
        return WEBRTC_VIDEO_CODEC_ERROR;
      }
    }
    if (!delivered)
      callback_->OnFrameDropped(frame.rtp_timestamp(), 0, true);
    return WEBRTC_VIDEO_CODEC_OK;
  }
  EncoderInfo GetEncoderInfo() const override {
    EncoderInfo info;
    info.implementation_name = "libvpx VP9 4:4:4";
    info.preferred_pixel_formats = {webrtc::VideoFrameBuffer::Type::kI444};
    info.scaling_settings = ScalingSettings::kOff;
    info.supports_simulcast = false;
    info.has_trusted_rate_controller = false;
    return info;
  }

 private:
  vpx_codec_ctx_t encoder_ = {};
  vpx_codec_enc_cfg_t config_ = {};
  webrtc::EncodedImageCallback* callback_ =
      nullptr;  // borrowed; calls are synchronous
  bool initialized_ = false, paused_ = false, keyframe_ = true;
  double fps_ = 30;
  int64_t timestamp_ = 0;
  int color_ = -1, range_ = -1;
};
}  // namespace
}  // namespace livekit_ffi
#endif

namespace livekit_ffi {
std::vector<webrtc::SdpVideoFormat> Vp9I444EncoderAdapter::SupportedFormats() {
#if defined(WEBRTC_WIN)
  return {webrtc::SdpVideoFormat(webrtc::SdpVideoFormat::VP9Profile1(),
                                 {webrtc::ScalabilityMode::kL1T1})};
#else
  return {};
#endif
}
std::unique_ptr<webrtc::VideoEncoder> Vp9I444EncoderAdapter::CreateEncoder(
    const webrtc::Environment&,
    const webrtc::SdpVideoFormat&) {
#if defined(WEBRTC_WIN)
  return std::make_unique<Vp9I444Encoder>();
#else
  return nullptr;
#endif
}
}  // namespace livekit_ffi
