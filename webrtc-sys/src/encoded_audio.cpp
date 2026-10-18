// Copyright 2026 LiveKit, Inc. Licensed under the Apache License, Version 2.0.
#include "livekit/encoded_audio.h"
#include "webrtc-sys/src/encoded_audio.rs.h"
#include "api/frame_transformer_interface.h"
#include "api/make_ref_counted.h"
#include "rtc_base/synchronization/mutex.h"
#include "rtc_base/time_utils.h"
#include <optional>
#include <stdexcept>

namespace livekit_ffi {
class AudioCopyTransformer : public webrtc::FrameTransformerInterface {
 public:
  explicit AudioCopyTransformer(rust::Box<EncodedAudioObserverWrapper> observer)
      : observer_(std::make_shared<rust::Box<EncodedAudioObserverWrapper>>(std::move(observer))) {}
  void Transform(std::unique_ptr<webrtc::TransformableFrameInterface> frame) override {
    webrtc::scoped_refptr<webrtc::TransformedFrameCallback> callback;
    std::shared_ptr<rust::Box<EncodedAudioObserverWrapper>> observer;
    {
      webrtc::MutexLock lock(&mutex_);
      callback = callback_;
      if (enabled_) observer = observer_;
    }
    if (!callback) return;  // The sender has already unregistered during teardown.
    std::optional<EncodedAudioFrame> packet;
    try {
      if (observer) {
        const auto data = frame->GetData();
        // Opus frames are small. Never allocate unbounded copies on a media thread.
        if (!data.empty() && data.size() <= 64 * 1024) {
          const auto capture = frame->CaptureTime();
          packet.emplace();
          packet->data.reserve(data.size());
          for (auto byte : data) packet->data.push_back(byte);
          packet->timestamp = frame->GetTimestamp();
          packet->ssrc = frame->GetSsrc();
          packet->payload_type = frame->GetPayloadType();
          packet->has_capture_time = capture.has_value();
          packet->capture_time_us = capture ? capture->us() : 0;
          packet->mime_type = rust::String(frame->GetMimeType());
        }
      }
    } catch (...) {
      packet.reset();
    }
    // The original frame is forwarded unmodified before invoking any recorder.
    callback->OnTransformedFrame(std::move(frame));
    if (packet) (*observer)->on_frame(std::move(*packet));
  }
  void RegisterTransformedFrameCallback(
      webrtc::scoped_refptr<webrtc::TransformedFrameCallback> callback) override {
    webrtc::MutexLock lock(&mutex_);
    callback_ = std::move(callback);
  }
  void UnregisterTransformedFrameCallback() override {
    webrtc::MutexLock lock(&mutex_);
    callback_ = nullptr;
  }
  void set_enabled(bool enabled) {
    webrtc::MutexLock lock(&mutex_);
    enabled_ = enabled;
  }
  void close() {
    std::shared_ptr<rust::Box<EncodedAudioObserverWrapper>> old;
    {
      webrtc::MutexLock lock(&mutex_);
      enabled_ = false;
      old.swap(observer_);
    }
  }
 private:
  webrtc::Mutex mutex_;
  bool enabled_ = false;
  webrtc::scoped_refptr<webrtc::TransformedFrameCallback> callback_;
  std::shared_ptr<rust::Box<EncodedAudioObserverWrapper>> observer_;
};
EncodedAudioObserver::EncodedAudioObserver(webrtc::scoped_refptr<AudioCopyTransformer> transformer)
    : transformer_(std::move(transformer)) {}
EncodedAudioObserver::~EncodedAudioObserver() { close(); }
void EncodedAudioObserver::set_enabled(bool enabled) const { transformer_->set_enabled(enabled); }
void EncodedAudioObserver::close() const { transformer_->close(); }
std::shared_ptr<EncodedAudioObserver> observe_encoded_audio(
    const RtpSender& sender, rust::Box<EncodedAudioObserverWrapper> observer) {
  if (sender.rtc_sender()->media_type() != webrtc::MediaType::AUDIO)
    throw std::runtime_error("Encoded audio observation requires an audio sender");
  auto transformer = webrtc::make_ref_counted<AudioCopyTransformer>(std::move(observer));
  sender.rtc_sender()->SetEncoderToPacketizerFrameTransformer(transformer);
  return std::make_shared<EncodedAudioObserver>(std::move(transformer));
}
int64_t audio_capture_clock_us() { return webrtc::TimeMicros(); }
}  // namespace livekit_ffi
