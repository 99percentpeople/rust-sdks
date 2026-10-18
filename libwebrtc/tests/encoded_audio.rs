#![cfg(not(target_arch = "wasm32"))]
use libwebrtc::{
    audio_frame::AudioFrame,
    audio_source::{native::NativeAudioSource, AudioSourceOptions},
    audio_stream::native::NativeAudioStream,
    media_stream_track::MediaStreamTrack,
    native::encoded_audio::EncodedAudioObserver,
    peer_connection::{
        AnswerOptions, IceGatheringState, OfferOptions, PeerConnection, PeerConnectionState,
    },
    peer_connection_factory::{
        native::PeerConnectionFactoryExt, ContinualGatheringPolicy, PeerConnectionFactory,
        RtcConfiguration,
    },
    rtp_transceiver::{RtpTransceiverDirection, RtpTransceiverInit},
    session_description::{SdpType, SessionDescription},
    stats::RtcStats,
};
use std::{
    borrow::Cow,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

struct Pair(PeerConnection, PeerConnection);
impl Drop for Pair {
    fn drop(&mut self) {
        for pc in [&self.0, &self.1] {
            pc.on_ice_candidate(None);
            pc.on_track(None);
            pc.close();
        }
    }
}

#[test]
fn invalid_sender_and_invalid_capture_release_partial_initialization() {
    for _ in 0..3 {
        let factory = PeerConnectionFactory::default();
        let pc = factory.create_peer_connection(RtcConfiguration::default()).unwrap();
        let sender = pc
            .add_transceiver_for_media(
                libwebrtc::MediaType::Video,
                RtpTransceiverInit {
                    direction: RtpTransceiverDirection::SendOnly,
                    stream_ids: vec![],
                    send_encodings: vec![],
                },
            )
            .unwrap()
            .sender();
        let owned = Arc::new(());
        let weak = Arc::downgrade(&owned);
        let observer = EncodedAudioObserver::new(
            &sender,
            Arc::new(move |_| {
                let _ = &owned;
            }),
        );
        assert!(observer.is_err());
        assert!(weak.upgrade().is_none());
        pc.close();
        let source = NativeAudioSource::new(AudioSourceOptions::default(), 48000, 1, 10);
        let frame = AudioFrame {
            data: Cow::Owned(vec![0; 480]),
            sample_rate: 48000,
            num_channels: 1,
            samples_per_channel: 480,
        };
        assert!(source.capture_frame_at(&frame, NativeAudioSource::capture_clock_us()).is_err());
        let source = NativeAudioSource::new(AudioSourceOptions::default(), 48000, 1, 0);
        assert!(source.capture_frame_at(&frame, -1).is_err());
    }
}
async fn gather(pc: &PeerConnection, description: SessionDescription) -> String {
    let candidates = Arc::new(Mutex::new(Vec::new()));
    let captured = candidates.clone();
    pc.on_ice_candidate(Some(Box::new(move |c| captured.lock().unwrap().push(c.to_string()))));
    let mut sdp = description.to_string();
    pc.set_local_description(description).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while pc.ice_gathering_state() != IceGatheringState::Complete {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    for c in candidates.lock().unwrap().iter() {
        sdp.push_str(&format!("a={}\r\n", c.trim()));
    }
    pc.on_ice_candidate(None);
    sdp
}
async fn feed(source: &NativeAudioSource, count: u32) {
    for n in 0..count {
        let data: Vec<i16> = (0..480)
            .map(|i| {
                ((f64::from(n * 480 + i) * 440.0 * std::f64::consts::TAU / 48000.0).sin() * 10000.0)
                    as i16
            })
            .collect();
        source
            .capture_frame_at(
                &AudioFrame {
                    data: Cow::Borrowed(&data),
                    sample_rate: 48000,
                    num_channels: 1,
                    samples_per_channel: 480,
                },
                NativeAudioSource::capture_clock_us(),
            )
            .unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
async fn received(pc: &PeerConnection) -> u64 {
    pc.get_stats()
        .await
        .unwrap()
        .into_iter()
        .filter_map(|s| match s {
            RtcStats::InboundRtp(s) => Some(s.inbound.total_samples_received),
            _ => None,
        })
        .sum()
}

#[tokio::test]
async fn sender_audio_observation_preserves_delivery_clocks_and_releases_callbacks_repeatedly() {
    for iteration in 0..3 {
        let factory = PeerConnectionFactory::default();
        let mut config = RtcConfiguration::default();
        config.continual_gathering_policy = ContinualGatheringPolicy::GatherOnce;
        let pair = Pair(
            factory.create_peer_connection(config.clone()).unwrap(),
            factory.create_peer_connection(config).unwrap(),
        );
        let source = NativeAudioSource::new(AudioSourceOptions::default(), 48000, 1, 0);
        let track = factory.create_audio_track("test", source.clone());
        let sender = pair
            .0
            .add_transceiver(
                MediaStreamTrack::Audio(track),
                RtpTransceiverInit {
                    direction: RtpTransceiverDirection::SendOnly,
                    stream_ids: vec!["test".into()],
                    send_encodings: vec![],
                },
            )
            .unwrap()
            .sender();
        let probe = Arc::new(AtomicUsize::new(0));
        let weak = Arc::downgrade(&probe);
        let copied = Arc::new(Mutex::new(Vec::new()));
        let output = copied.clone();
        let observer = EncodedAudioObserver::new(
            &sender,
            Arc::new(move |frame| {
                probe.fetch_add(1, Ordering::Relaxed);
                output.lock().unwrap().push(frame);
            }),
        )
        .unwrap();
        // Cancellation before SDP must release the Rust callback even if the sender is alive.
        if iteration == 0 {
            observer.close();
            assert!(weak.upgrade().is_none());
            drop(observer);
            continue;
        }
        let sinks = Arc::new(Mutex::new(Vec::new()));
        let sink = sinks.clone();
        pair.1.on_track(Some(Box::new(move |event| {
            if let MediaStreamTrack::Audio(track) = event.track {
                sink.lock().unwrap().push(NativeAudioStream::new(track, 48000, 1));
            }
        })));
        let offer =
            gather(&pair.0, pair.0.create_offer(OfferOptions::default()).await.unwrap()).await;
        pair.1
            .set_remote_description(SessionDescription::parse(&offer, SdpType::Offer).unwrap())
            .await
            .unwrap();
        let answer =
            gather(&pair.1, pair.1.create_answer(AnswerOptions::default()).await.unwrap()).await;
        pair.0
            .set_remote_description(SessionDescription::parse(&answer, SdpType::Answer).unwrap())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            while pair.0.connection_state() != PeerConnectionState::Connected {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        feed(&source, 20).await;
        assert!(copied.lock().unwrap().is_empty());
        let before = received(&pair.1).await;
        observer.set_enabled(true);
        let start = NativeAudioSource::capture_clock_us();
        feed(&source, 50).await;
        {
            let frames = copied.lock().unwrap();
            assert!(frames.len() >= 10, "No real encoded audio copies");
            for frame in frames.iter() {
                assert!(!frame.data.is_empty());
                assert_eq!(frame.mime_type.to_lowercase(), "audio/opus");
                assert!(frame.capture_time_us.is_some_and(
                    |t| t >= start - 50_000 && t <= NativeAudioSource::capture_clock_us()
                ));
            }
            assert!(frames.windows(2).all(|w| w[1].timestamp.wrapping_sub(w[0].timestamp) == 960));
        }
        let during = received(&pair.1).await;
        assert!(during > before + 10000, "Original receiver stopped during copying");
        observer.set_enabled(false);
        tokio::time::sleep(Duration::from_millis(30)).await;
        let count = copied.lock().unwrap().len();
        feed(&source, 20).await;
        assert_eq!(copied.lock().unwrap().len(), count);
        assert!(received(&pair.1).await > during);
        observer.set_enabled(true);
        feed(&source, 20).await;
        assert!(copied.lock().unwrap().len() > count);
        observer.close();
        drop(observer);
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(weak.upgrade().is_none(), "Native callback leaked after close");
        let before_close = received(&pair.1).await;
        feed(&source, 20).await;
        assert!(received(&pair.1).await > before_close, "Closing observation stopped sender");
        drop(pair);
        drop(sinks);
        assert!(weak.upgrade().is_none());
    }
}
