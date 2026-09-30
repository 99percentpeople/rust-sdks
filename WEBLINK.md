# Weblink native WebRTC branch

This branch carries Weblink's native screen-sharing changes for Cargo Git
dependencies. It is based on upstream `2dd762da4fdc8b73504983aff078d7824ea5d4ee`.
`libwebrtc` retains version 0.3.49 and its release metadata because Weblink pins
that version; its executable sources are identical to the 0.3.50 release before
these patches. `webrtc-sys` remains 0.3.47 and `webrtc-sys-build` remains 0.3.19.
The build helper and engine download/version logic are unchanged.

Consumers must pin a full Git commit in Cargo.toml and commit Cargo.lock. Do not
track the branch tip implicitly. Upstream license headers and notices remain.

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

The patch changes only these upstream files:

- `libwebrtc/src/peer_connection_factory.rs`
- `libwebrtc/src/native/peer_connection_factory.rs`
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
