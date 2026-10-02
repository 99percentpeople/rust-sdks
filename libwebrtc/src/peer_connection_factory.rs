// Copyright 2025 LiveKit, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::fmt::Debug;

use crate::{
    imp::peer_connection_factory as imp_pcf, peer_connection::PeerConnection,
    rtp_parameters::RtpCapabilities, MediaType, RtcError,
};

#[derive(Debug, Clone)]
pub struct IceServer {
    pub urls: Vec<String>,
    pub username: String,
    pub password: String,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ContinualGatheringPolicy {
    GatherOnce,
    GatherContinually,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum IceTransportsType {
    Relay,
    NoHost,
    All,
}

/// Configuration for a [`PeerConnection`].
///
/// This type is `#[non_exhaustive]`: construct it from [`RtcConfiguration::default`]
/// and set the fields you need, e.g.
/// ```
/// # use libwebrtc::peer_connection_factory::{IceTransportsType, RtcConfiguration};
/// let mut cfg = RtcConfiguration::default();
/// cfg.ice_transport_type = IceTransportsType::Relay;
/// ```
/// New fields may be added in future releases without a breaking change.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RtcConfiguration {
    pub ice_servers: Vec<IceServer>,
    pub continual_gathering_policy: ContinualGatheringPolicy,
    pub ice_transport_type: IceTransportsType,
    /// WARP: enable SNAP (SCTP-INIT-in-SDP). Maps to the immutable
    /// `enable_sctp_snap` RTCConfiguration field, so it must be set the same at
    /// PeerConnection creation and every set_configuration.
    pub enable_sctp_snap: bool,
}

impl Default for RtcConfiguration {
    fn default() -> Self {
        Self {
            ice_servers: vec![],
            continual_gathering_policy: ContinualGatheringPolicy::GatherContinually,
            ice_transport_type: IceTransportsType::All,
            enable_sctp_snap: false,
        }
    }
}

#[derive(Clone, Default)]
pub struct PeerConnectionFactory {
    pub(crate) handle: imp_pcf::PeerConnectionFactory,
}

impl Debug for PeerConnectionFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("PeerConnectionFactory").finish()
    }
}

/// Per-factory video sender policy. Defaults preserve upstream behavior.
#[derive(Debug, Clone, Copy, Default)]
pub struct VideoSendOptions {
    /// Minimum RTP playout delay; nonzero requires an explicit maximum.
    pub min_playout_delay_ms: u32,
    pub max_playout_delay_ms: Option<u32>,
    pub pacing_factor: Option<f32>,
    /// Let WebRTC schedule frame drops for single-stream software H.264.
    /// OpenH264 still controls quantization; its internal consecutive frame
    /// skipping is disabled. Congestion control and WebRTC's dropper stay on.
    pub software_h264_external_frame_dropper: bool,
}

impl PeerConnectionFactory {
    /// Creates a native factory with optional video sender timing hints.
    ///
    /// `max_playout_delay_ms` sends an RTP playout-delay range of 0..max when
    /// negotiated. The receiver applies it on a best-effort basis. Values must
    /// be multiples of 10 ms in 0..=40950. `pacing_factor` controls packet burst
    /// pacing relative to the bandwidth estimate (1.0..=2.5), without changing
    /// encoder bitrate limits or disabling congestion control. None preserves
    /// WebRTC's defaults. Options are scoped to this factory.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_video_send_timing(
        max_playout_delay_ms: Option<u32>,
        pacing_factor: Option<f32>,
    ) -> Result<Self, RtcError> {
        Self::with_video_send_options(VideoSendOptions {
            max_playout_delay_ms,
            pacing_factor,
            ..Default::default()
        })
    }

    /// Creates a native factory with explicit sender and software H.264 policy.
    /// Timing ranges follow [`Self::with_video_send_timing`]. The minimum may
    /// also be set in 10 ms units, and must not exceed the explicit maximum.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_video_send_options(options: VideoSendOptions) -> Result<Self, RtcError> {
        Self::with_video_send_policy(options, false, false)
    }

    /// Advertises H.265 Main 8-bit for externally encoded native frames in this factory.
    /// Call only after verifying an external HEVC encoder is available, and set
    /// each HEVC sender to [`crate::rtp_sender::VideoEncoderBackend::PreEncoded`].
    /// This does not provide a software encoder or an HEVC decoder.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_external_hevc_video_send_options(
        options: VideoSendOptions,
    ) -> Result<Self, RtcError> {
        Self::with_video_send_policy(options, true, false)
    }

    /// Creates a screen sender with periodic bandwidth probes during low activity.
    /// Probes allow recovery after idle or congestion without imposing a minimum
    /// bitrate. Pacing, feedback and the sender's maximum bitrate remain active.
    /// Set `external_hevc` only with an available external encoder, as described
    /// by [`Self::with_external_hevc_video_send_options`].
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_screen_video_send_options(
        options: VideoSendOptions,
        external_hevc: bool,
    ) -> Result<Self, RtcError> {
        Self::with_video_send_policy(options, external_hevc, true)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn with_video_send_policy(
        options: VideoSendOptions,
        external_hevc: bool,
        periodic_alr_probing: bool,
    ) -> Result<Self, RtcError> {
        let VideoSendOptions { min_playout_delay_ms, max_playout_delay_ms, pacing_factor, .. } =
            options;
        if min_playout_delay_ms % 10 != 0
            || min_playout_delay_ms > max_playout_delay_ms.unwrap_or(0)
            || max_playout_delay_ms.is_some_and(|ms| ms > 40950 || ms % 10 != 0)
            || pacing_factor
                .is_some_and(|factor| !factor.is_finite() || !(1.0..=2.5).contains(&factor))
        {
            return Err(RtcError {
                error_type: crate::RtcErrorType::Internal,
                message: "Invalid video sender timing options".into(),
            });
        }
        Ok(Self {
            handle: imp_pcf::PeerConnectionFactory::with_video_send_policy(
                options,
                external_hevc,
                periodic_alr_probing,
            ),
        })
    }

    /// Creates a native peer connection factory that renders received video as soon as possible.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_zero_playout_delay() -> Self {
        Self { handle: imp_pcf::PeerConnectionFactory::with_zero_playout_delay() }
    }

    /// Creates a native peer connection factory with the given runtime options.
    /// `zero_playout_delay` and `enable_warp` (SPED + SNAP) are independent and
    /// may be combined.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_options(zero_playout_delay: bool, enable_warp: bool) -> Self {
        Self {
            handle: imp_pcf::PeerConnectionFactory::with_options(zero_playout_delay, enable_warp),
        }
    }

    pub fn create_peer_connection(
        &self,
        config: RtcConfiguration,
    ) -> Result<PeerConnection, RtcError> {
        self.handle.create_peer_connection(config)
    }

    pub fn get_rtp_sender_capabilities(&self, media_type: MediaType) -> RtpCapabilities {
        self.handle.get_rtp_sender_capabilities(media_type)
    }

    pub fn get_rtp_receiver_capabilities(&self, media_type: MediaType) -> RtpCapabilities {
        self.handle.get_rtp_receiver_capabilities(media_type)
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::{PeerConnectionFactory, VideoSendOptions};

    #[test]
    fn screen_probing_is_factory_local_and_does_not_change_default_policy() {
        for _ in 0..3 {
            let ordinary =
                PeerConnectionFactory::with_video_send_options(VideoSendOptions::default())
                    .unwrap();
            let screen = PeerConnectionFactory::with_screen_video_send_options(
                VideoSendOptions::default(),
                false,
            )
            .unwrap();
            assert!(!ordinary.handle.periodic_alr_probing_enabled());
            assert!(screen.handle.periodic_alr_probing_enabled());
            drop(screen);
            assert!(!ordinary.handle.periodic_alr_probing_enabled());
        }
    }

    #[test]
    fn zero_playout_delay_factory_uses_force_playout_delay_field_trial() {
        let default_factory = PeerConnectionFactory::default();
        assert!(!default_factory.handle.zero_playout_delay_enabled());
        drop(default_factory);

        let low_latency_factory = PeerConnectionFactory::with_zero_playout_delay();
        assert!(low_latency_factory.handle.zero_playout_delay_enabled());
    }
}

pub mod native {
    use super::PeerConnectionFactory;
    use crate::{
        audio_source::native::NativeAudioSource, audio_track::RtcAudioTrack,
        video_source::native::NativeVideoSource, video_track::RtcVideoTrack,
    };

    pub trait PeerConnectionFactoryExt {
        fn create_video_track(&self, label: &str, source: NativeVideoSource) -> RtcVideoTrack;
        fn create_audio_track(&self, label: &str, source: NativeAudioSource) -> RtcAudioTrack;

        /// Create an audio track that uses the Platform ADM for capture.
        /// The track will capture audio from the selected recording device.
        fn create_device_audio_track(&self, label: &str) -> RtcAudioTrack;

        // Device enumeration
        fn playout_devices(&self) -> i16;
        fn recording_devices(&self) -> i16;
        fn playout_device_name(&self, index: u16) -> String;
        fn recording_device_name(&self, index: u16) -> String;
        /// Get device GUID (platform-specific unique identifier, stable across hot-plug)
        fn playout_device_guid(&self, index: u16) -> String;
        fn recording_device_guid(&self, index: u16) -> String;

        // Device selection by index
        fn set_playout_device(&self, index: u16) -> bool;
        fn set_recording_device(&self, index: u16) -> bool;
        /// Device selection by GUID (preferred - stable across device changes)
        fn set_playout_device_by_guid(&self, guid: &str) -> bool;
        fn set_recording_device_by_guid(&self, guid: &str) -> bool;

        // Recording control (for device switching while active)
        fn stop_recording(&self) -> bool;
        fn init_recording(&self) -> bool;
        fn start_recording(&self) -> bool;
        fn recording_is_initialized(&self) -> bool;

        // Playout control (for device switching while active)
        fn stop_playout(&self) -> bool;
        fn init_playout(&self) -> bool;
        fn start_playout(&self) -> bool;
        fn playout_is_initialized(&self) -> bool;

        // Built-in audio processing (hardware AEC/AGC/NS)
        // Only available on iOS and some Android devices
        fn builtin_aec_is_available(&self) -> bool;
        fn builtin_agc_is_available(&self) -> bool;
        fn builtin_ns_is_available(&self) -> bool;
        fn enable_builtin_aec(&self, enable: bool) -> bool;
        fn enable_builtin_agc(&self, enable: bool) -> bool;
        fn enable_builtin_ns(&self, enable: bool) -> bool;

        // ADM recording control
        // Use this to disable microphone when only using NativeAudioSource
        fn set_adm_recording_enabled(&self, enabled: bool);
        fn adm_recording_enabled(&self) -> bool;

        // ADM playout control
        // When disabled (default), playout uses synthetic mode - remote audio is
        // delivered via FFI callbacks. When enabled, plays through platform speakers.
        fn set_adm_playout_enabled(&self, enabled: bool);
        fn adm_playout_enabled(&self) -> bool;

        // Platform ADM lifecycle management
        // Call acquire_platform_adm when creating PlatformAudio.
        // Call release_platform_adm when disposing PlatformAudio.
        // The Platform ADM is only created when first acquired, and terminated
        // when the last reference is released.
        fn acquire_platform_adm(&self) -> bool;
        fn release_platform_adm(&self);
        fn platform_adm_ref_count(&self) -> i32;
        fn is_platform_adm_active(&self) -> bool;
    }

    impl PeerConnectionFactoryExt for PeerConnectionFactory {
        fn create_video_track(&self, label: &str, source: NativeVideoSource) -> RtcVideoTrack {
            self.handle.create_video_track(label, source)
        }

        fn create_audio_track(&self, label: &str, source: NativeAudioSource) -> RtcAudioTrack {
            self.handle.create_audio_track(label, source)
        }

        fn create_device_audio_track(&self, label: &str) -> RtcAudioTrack {
            self.handle.create_device_audio_track(label)
        }

        fn playout_devices(&self) -> i16 {
            self.handle.playout_devices()
        }

        fn recording_devices(&self) -> i16 {
            self.handle.recording_devices()
        }

        fn playout_device_name(&self, index: u16) -> String {
            self.handle.playout_device_name(index)
        }

        fn recording_device_name(&self, index: u16) -> String {
            self.handle.recording_device_name(index)
        }

        fn playout_device_guid(&self, index: u16) -> String {
            self.handle.playout_device_guid(index)
        }

        fn recording_device_guid(&self, index: u16) -> String {
            self.handle.recording_device_guid(index)
        }

        fn set_playout_device(&self, index: u16) -> bool {
            self.handle.set_playout_device(index)
        }

        fn set_recording_device(&self, index: u16) -> bool {
            self.handle.set_recording_device(index)
        }

        fn set_playout_device_by_guid(&self, guid: &str) -> bool {
            self.handle.set_playout_device_by_guid(guid)
        }

        fn set_recording_device_by_guid(&self, guid: &str) -> bool {
            self.handle.set_recording_device_by_guid(guid)
        }

        fn stop_recording(&self) -> bool {
            self.handle.stop_recording()
        }

        fn init_recording(&self) -> bool {
            self.handle.init_recording()
        }

        fn start_recording(&self) -> bool {
            self.handle.start_recording()
        }

        fn recording_is_initialized(&self) -> bool {
            self.handle.recording_is_initialized()
        }

        fn stop_playout(&self) -> bool {
            self.handle.stop_playout()
        }

        fn init_playout(&self) -> bool {
            self.handle.init_playout()
        }

        fn start_playout(&self) -> bool {
            self.handle.start_playout()
        }

        fn playout_is_initialized(&self) -> bool {
            self.handle.playout_is_initialized()
        }

        fn builtin_aec_is_available(&self) -> bool {
            self.handle.builtin_aec_is_available()
        }

        fn builtin_agc_is_available(&self) -> bool {
            self.handle.builtin_agc_is_available()
        }

        fn builtin_ns_is_available(&self) -> bool {
            self.handle.builtin_ns_is_available()
        }

        fn enable_builtin_aec(&self, enable: bool) -> bool {
            self.handle.enable_builtin_aec(enable)
        }

        fn enable_builtin_agc(&self, enable: bool) -> bool {
            self.handle.enable_builtin_agc(enable)
        }

        fn enable_builtin_ns(&self, enable: bool) -> bool {
            self.handle.enable_builtin_ns(enable)
        }

        fn set_adm_recording_enabled(&self, enabled: bool) {
            self.handle.set_adm_recording_enabled(enabled)
        }

        fn adm_recording_enabled(&self) -> bool {
            self.handle.adm_recording_enabled()
        }

        fn set_adm_playout_enabled(&self, enabled: bool) {
            self.handle.set_adm_playout_enabled(enabled)
        }

        fn adm_playout_enabled(&self) -> bool {
            self.handle.adm_playout_enabled()
        }

        fn acquire_platform_adm(&self) -> bool {
            self.handle.acquire_platform_adm()
        }

        fn release_platform_adm(&self) {
            self.handle.release_platform_adm()
        }

        fn platform_adm_ref_count(&self) -> i32 {
            self.handle.platform_adm_ref_count()
        }

        fn is_platform_adm_active(&self) -> bool {
            self.handle.is_platform_adm_active()
        }
    }
}
