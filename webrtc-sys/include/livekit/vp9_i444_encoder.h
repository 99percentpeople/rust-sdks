// Copyright 2026 LiveKit, Inc.
// SPDX-License-Identifier: Apache-2.0
#pragma once
#include "api/video_codecs/video_encoder_factory_template.h"

namespace livekit_ffi {
// Single-layer SDR profile 1, using the libvpx bundled with the WebRTC engine.
struct Vp9I444EncoderAdapter {
  static std::vector<webrtc::SdpVideoFormat> SupportedFormats();
  static std::unique_ptr<webrtc::VideoEncoder> CreateEncoder(
      const webrtc::Environment&,
      const webrtc::SdpVideoFormat&);
  static bool IsScalabilityModeSupported(webrtc::ScalabilityMode mode) {
    return mode == webrtc::ScalabilityMode::kL1T1;
  }
};
}  // namespace livekit_ffi
