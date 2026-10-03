# Weblink native WebRTC branch

This branch carries Weblink's native screen-sharing changes for Cargo Git
dependencies. It is based on upstream `ccfb8770f26049b8c50a2b760c91eff682f043b9`.
`libwebrtc` retains version 0.3.49 and its release metadata because Weblink pins
that version; its executable sources are identical to the 0.3.50 release before
these patches. `webrtc-sys` remains 0.3.47 and `webrtc-sys-build` remains 0.3.19.
The build helper and engine download/version logic are unchanged.

Consumers must pin a full Git commit in Cargo.toml and commit Cargo.lock. Do not
track the branch tip implicitly. Upstream license headers and notices remain.

Keep only two long-lived branches: `main` follows the official LiveKit repository,
and `weblink-native-media` carries all Weblink patches. When updating upstream,
advance `main`, then rebase the complete Weblink patch series onto it so local
changes always follow the official commits in a linear history. Verify the
rebased series with `git range-diff`, publish with an explicit force-with-lease
against the previously observed maintenance tip, and refresh the consumer's
pinned Git revision and lockfile after validation. Do not create feature branches
for individual Weblink patches.

Local changes expose `PeerConnectionFactory::with_video_send_options` and the
timing-only convenience method `with_video_send_timing`:

- A negotiated RTP playout-delay hint (configurable minimum and maximum in 10 ms
  units), using `WebRTC-ForceSendPlayoutDelay`. The minimum defaults to zero;
  a positive minimum requires an explicit maximum at least as large.
- An optional video pacing factor using `WebRTC-Video-Pacing`.
- Optional frame-drop scheduling by WebRTC for single-stream software H.264 in
  real-time mode. Only the encoder's codec-settings copy disables OpenH264's
  internal frame skipping. The outer WebRTC dropper remains enabled, including
  its untrusted-rate-controller flag. OpenH264 quantization, bitrate adjustment,
  congestion control and pacing remain active. Hardware, other codecs and
  multi-stream/screen-content configurations retain their defaults.

Configuration belongs to the factory's `Environment`, not process-global field
trials. Existing constructors retain their original behavior. Rust validates
the public parameters before entering C++; the application selects its policy
in `crates/desktop-capture/src/media/windows.rs`. These settings preserve WebRTC
bandwidth estimation, retransmission and congestion control. The playout hint
does not impose a network deadline or guarantee end-to-end latency.

`with_screen_video_send_options` additionally enables periodic ALR bandwidth
probing using `WebRTC-VideoRateControl`. The application's hardware pass-through
and real-time H264 sources use camera mode and otherwise miss screenshare's
default probes when activity falls below the estimated capacity. This opt-in
allows WebRTC to probe for recovery without a forced bitrate floor, changing the
codec preset, or replacing the chosen pacing policy. Default constructors keep
their prior behavior. Validate idle-to-motion and congestion recovery separately
from actual encoded bitrate: easy content may remain far below the budget.

`PeerConnection::set_max_bitrate` sets the native transport's total media ceiling,
separately from each RTP encoding's maximum. Apply it before negotiation and when
changing the budget, including any audio allocation. With no finite transport
ceiling, the bundled engine's `ProbeController` defaults to a 5 Mbps probe limit;
an RTP encoding limit alone does not replace that fallback. This is a limit on
probing, not a hard cap on achieved media throughput. The API changes only the
maximum: it neither imposes a minimum rate nor resets the starting estimate.
GCC can still reduce the target in response to congestion. Periodic ALR probing
retains the engine's default cadence and scale.

The patch changes only these upstream files:

- `libwebrtc/src/peer_connection_factory.rs`
- `libwebrtc/src/peer_connection.rs`
- `libwebrtc/src/native/peer_connection.rs`
- `libwebrtc/src/native/peer_connection_factory.rs`
- `webrtc-sys/src/peer_connection.rs`
- `webrtc-sys/src/peer_connection.cpp`
- `webrtc-sys/include/livekit/peer_connection.h`
- `webrtc-sys/src/peer_connection_factory.rs`
- `webrtc-sys/src/peer_connection_factory.cpp`
- `webrtc-sys/include/livekit/peer_connection_factory.h`
- `webrtc-sys/src/video_encoder_factory.cpp`
- `webrtc-sys/include/livekit/video_encoder_factory.h`

On upgrades, compare these files against the corresponding published crates,
reapply the sender options and scoped software H.264 policy, and validate
native-to-browser RTP with an ordinary browser (no forced receiver field trials).
Include scene changes, reduced bitrate limits and recovery; smooth output must
not be obtained by bypassing congestion control. Remove the local forks when an
upstream release exposes equivalent per-factory options.

`PeerConnectionFactory::with_external_hevc_video_send_options` additionally opts
into advertising the H265 Main 8-bit pass-through format for a verified application-owned
encoder. The caller must select `VideoEncoderBackend::PreEncoded` for that sender.
This is factory-local and adds neither a software encoder nor a decoder. Default
factories, capability queries for software and ordinary sender options keep their
existing behavior. Validate repeated factory/session creation, cancelled offers,
real HEVC RTP decode, keyframe recovery and live settings on supported hardware.

The pre-encoded backend reports a trusted external rate controller, disabling
WebRTC's encoder-input rate dropper for these already-compressed frames. Dropping
them after external encoding can break inter-frame references. Applications must
honor `take_rate_control_request`, measure actual output, and skip raw input before
encoding when the hardware overshoots its accepted target. This applies to every
pass-through codec, including H264 and H265; software encoders keep their policies.
Bandwidth feedback, RTP pacing and transport congestion control remain active.
When upgrading, additionally compare `webrtc-sys/src/passthrough_video_encoder.cpp`
and `libwebrtc/src/native/video_source.rs`, and test full-frame motion under both
constrained and sufficient bitrate budgets.

External rate feedback binds to the source's mailbox on the first encoded frame.
Subsequent `SetRates` calls publish directly, including zero-rate suspension and
positive-rate recovery, without waiting for another frame. The pass-through keeps
only a weak mailbox reference and clears it on release; it retains no frame payload.
`NativeVideoSource::set_rate_control_wakeup` optionally notifies a worker to drain
the latest request. Callbacks run outside encoder/mailbox locks, must stay short,
and must not strongly capture their owning source. Clear the callback on every
worker exit; an already-running notification may finish afterwards. Use one encoded
source per sender. This does not change transport estimation, probing or pacing.
On upgrades also compare `encoded_video_frame_buffer.{h,cpp}`, `video_track.{h,cpp,rs}`
and the public `libwebrtc/src/video_source.rs` wrapper.
