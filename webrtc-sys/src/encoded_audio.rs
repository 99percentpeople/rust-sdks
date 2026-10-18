// Copyright 2026 LiveKit, Inc. Licensed under the Apache License, Version 2.0.
use std::sync::Arc;

#[cxx::bridge(namespace = "livekit_ffi")]
pub mod ffi {
    /// A copy of one sender audio payload, before RTP packetization.
    struct EncodedAudioFrame {
        data: Vec<u8>,
        timestamp: u32,
        ssrc: u32,
        payload_type: u8,
        capture_time_us: i64,
        has_capture_time: bool,
        mime_type: String,
    }
    unsafe extern "C++" {
        include!("livekit/encoded_audio.h");
        type RtpSender = crate::rtp_sender::ffi::RtpSender;
        type EncodedAudioObserver;
        fn observe_encoded_audio(
            sender: &RtpSender,
            observer: Box<EncodedAudioObserverWrapper>,
        ) -> Result<SharedPtr<EncodedAudioObserver>>;
        fn set_enabled(self: &EncodedAudioObserver, enabled: bool);
        fn close(self: &EncodedAudioObserver);
        fn audio_capture_clock_us() -> i64;
    }
    extern "Rust" {
        type EncodedAudioObserverWrapper;
        fn on_frame(self: &EncodedAudioObserverWrapper, frame: EncodedAudioFrame);
    }
}

/// Owns the application callback without retaining the sender or native observer.
pub struct EncodedAudioObserverWrapper(pub Arc<dyn Fn(ffi::EncodedAudioFrame) + Send + Sync>);
impl EncodedAudioObserverWrapper {
    /// Callback failure must not unwind through the native media thread.
    fn on_frame(&self, frame: ffi::EncodedAudioFrame) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (self.0)(frame)));
    }
}
crate::impl_thread_safety!(ffi::EncodedAudioObserver, Send + Sync);
