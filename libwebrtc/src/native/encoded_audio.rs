// Copyright 2026 LiveKit, Inc. Licensed under the Apache License, Version 2.0.
//! Optional sender audio copies; recording consumers must never block the media thread.
use crate::{rtp_sender::RtpSender, RtcError, RtcErrorType};
use cxx::SharedPtr;
use std::sync::Arc;
use webrtc_sys::encoded_audio::{self as sys, ffi};

/// One sender payload, retaining the original RTP and capture timestamps.
pub struct EncodedAudioFrame {
    pub data: Vec<u8>,
    pub timestamp: u32,
    pub ssrc: u32,
    pub payload_type: u8,
    /// Microseconds on the same clock as [`crate::audio_source::native::NativeAudioSource::capture_clock_us`].
    pub capture_time_us: Option<i64>,
    pub mime_type: String,
}
/// Owns a pass-through audio observation; drop disables copies without detaching the transform.
pub struct EncodedAudioObserver {
    handle: SharedPtr<ffi::EncodedAudioObserver>,
}
impl EncodedAudioObserver {
    /// Install before negotiation, on a sender without an existing transform/cryptor.
    /// The callback runs after network forwarding and must only enqueue bounded work.
    pub fn new(
        sender: &RtpSender,
        observer: Arc<dyn Fn(EncodedAudioFrame) + Send + Sync>,
    ) -> Result<Self, RtcError> {
        let callback = sys::EncodedAudioObserverWrapper(Arc::new(move |frame| {
            observer(EncodedAudioFrame {
                data: frame.data,
                timestamp: frame.timestamp,
                ssrc: frame.ssrc,
                payload_type: frame.payload_type,
                capture_time_us: frame.has_capture_time.then_some(frame.capture_time_us),
                mime_type: frame.mime_type,
            });
        }));
        let handle =
            ffi::observe_encoded_audio(&sender.handle.sys_handle, Box::new(callback)).map_err(
                |e| RtcError { error_type: RtcErrorType::InvalidState, message: e.to_string() },
            )?;
        Ok(Self { handle })
    }
    /// Enable or pause copying; newly installed observers start disabled.
    pub fn set_enabled(&self, enabled: bool) {
        self.handle.set_enabled(enabled);
    }
    /// Permanently release the callback, leaving the original sender running.
    pub fn close(&self) {
        self.handle.close();
    }
}
impl Drop for EncodedAudioObserver {
    fn drop(&mut self) {
        self.close();
    }
}
