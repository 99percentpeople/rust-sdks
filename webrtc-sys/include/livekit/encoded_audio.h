// Copyright 2026 LiveKit, Inc. Licensed under the Apache License, Version 2.0.
#pragma once
#include <memory>
#include "api/scoped_refptr.h"
#include "livekit/rtp_sender.h"
#include "rust/cxx.h"

namespace livekit_ffi {
struct EncodedAudioObserverWrapper;
class AudioCopyTransformer;

// Dropping the observer disables copying; the sender's pass-through transform
// stays installed until its normal connection teardown.
class EncodedAudioObserver {
 public:
  explicit EncodedAudioObserver(webrtc::scoped_refptr<AudioCopyTransformer> transformer);
  ~EncodedAudioObserver();
  void set_enabled(bool enabled) const;
  void close() const;
 private:
  webrtc::scoped_refptr<AudioCopyTransformer> transformer_;
};
std::shared_ptr<EncodedAudioObserver> observe_encoded_audio(
    const RtpSender& sender, rust::Box<EncodedAudioObserverWrapper> observer);
int64_t audio_capture_clock_us();
}  // namespace livekit_ffi
