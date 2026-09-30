use super::*;
#[test]
fn external_screen_frames_use_the_real_h264_pipeline_without_camera_or_audio_changes() {
    let e = Engine::create(false);
    e.configure(true, false, 500_000, 1);
    e.set_external_video(true);
    assert!(e.shared.settings().unwrap().muted);
    e.configure(true, true, 500_000, 1);
    let worker = {
        let state = e.shared.clone();
        thread::spawn(move || run_camera(state))
    };
    e.submit_video_frame(64, 32, [40, 120, 220, 255].repeat(64 * 32));
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut encoded = None;
    while encoded.is_none() && Instant::now() < deadline {
        for event in e.poll() {
            if let DesktopCallEvent::Encoded {
                kind: 2,
                data,
                key_frame,
                ..
            } = event
            {
                encoded = Some((data, key_frame));
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
    e.stop();
    worker.join().unwrap();
    let (data, key) = encoded.expect("screen pixels must reach H.264 output");
    assert!(key);
    let mut decoder = openh264::decoder::Decoder::new().unwrap();
    let frame = decoder
        .decode(&data)
        .unwrap()
        .expect("normal call decoder accepts screen share");
    use openh264::formats::YUVSource;
    assert_eq!(frame.dimensions(), (64, 32));
    assert!(e.shared.settings().unwrap().muted);
    assert!(e.poll().is_empty());
}

#[test]
fn screen_source_switch_discards_old_pixels_and_rejects_invalid_or_late_frames() {
    let e = Engine::create(false);
    e.configure(false, true, 150_000, 1);
    let camera_generation = e.shared.settings().unwrap().generation;
    e.shared.emit(
        camera_generation,
        DesktopCallEvent::Video {
            local: true,
            width: 2,
            height: 2,
            rgba: vec![0; 16],
        },
    );
    e.set_external_video(true);
    assert!(e.poll().is_empty());
    e.submit_video_frame(2, 2, vec![1; 15]);
    assert!(e.shared.external_frame.lock().unwrap().is_none());
    e.submit_video_frame(2, 2, vec![1; 16]);
    e.submit_video_frame(2, 2, vec![2; 16]);
    assert_eq!(
        e.shared
            .external_frame
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .as_raw(),
        &vec![2; 12]
    );
    e.set_external_video(false);
    e.submit_video_frame(2, 2, vec![3; 16]);
    assert!(e.shared.external_frame.lock().unwrap().is_none());
    assert!(!e.shared.settings().unwrap().muted);
    e.set_external_video(true);
    e.stop();
    e.submit_video_frame(2, 2, vec![4; 16]);
    assert!(e.shared.external_frame.lock().unwrap().is_none());
}
#[test]
fn audio_switch_discards_old_capture_and_rejects_stale_device_choices() {
    let e = Engine::create(false);
    e.configure(false, false, 150_000, 1);
    let options = vec![
        super::super::DesktopAudioDevice {
            id: String::new(),
            name: "System default".into(),
        },
        super::super::DesktopAudioDevice {
            id: "headset".into(),
            name: "Headset".into(),
        },
    ];
    *e.shared.audio_devices.lock().unwrap() = DesktopAudioDevices {
        microphones: options.clone(),
        speakers: options,
        ..Default::default()
    };
    let old = e.shared.settings().unwrap().generation;
    e.shared.emit(
        old,
        DesktopCallEvent::Encoded {
            kind: 1,
            timestamp_us: 0,
            key_frame: false,
            data: vec![1],
        },
    );
    e.shared.force_key.store(false, Ordering::Release);
    e.shared.emit(
        old,
        DesktopCallEvent::Encoded {
            kind: 2,
            timestamp_us: 0,
            key_frame: false,
            data: vec![2],
        },
    );
    e.select_audio_devices("headset".into(), String::new());
    assert!(
        e.shared.force_key.load(Ordering::Acquire),
        "discarding queued video requires a new reference frame"
    );
    assert!(
        e.poll().is_empty(),
        "old microphone audio must not escape after selection"
    );
    assert!(
        !e.shared.stopped.load(Ordering::Acquire),
        "switch must preserve call transport"
    );
    assert_eq!(e.shared.audio_selection.lock().unwrap().revision, 1);
    e.select_audio_devices("headset".into(), String::new());
    e.select_audio_devices("unplugged".into(), String::new());
    assert_eq!(e.shared.audio_selection.lock().unwrap().revision, 1);
    assert_eq!(
        e.shared.audio_selection.lock().unwrap().microphone,
        "headset"
    );
    e.shared.audio_devices.lock().unwrap().error = Some("Audio switched to system default.".into());
    e.select_audio_devices("headset".into(), String::new());
    assert_eq!(
        e.shared.audio_selection.lock().unwrap().revision,
        2,
        "a device that failed to open can be selected again"
    );
}
#[test]
fn cameras_without_mjpeg_or_720p_use_an_advertised_decodable_mode() {
    use nokhwa::utils::Resolution;
    let vga = CameraFormat::new(Resolution::new(640, 480), FrameFormat::YUYV, 30);
    assert_eq!(camera_format(&[vga]), Some(vga));
    let hd = CameraFormat::new(Resolution::new(1280, 720), FrameFormat::MJPEG, 30);
    let huge = CameraFormat::new(Resolution::new(3840, 2160), FrameFormat::MJPEG, 30);
    let slow = CameraFormat::new(Resolution::new(1280, 720), FrameFormat::MJPEG, 5);
    assert_eq!(camera_format(&[huge, slow, vga, hd]), Some(hd));
    assert_eq!(camera_format(&[slow, vga]), Some(vga));
    assert_eq!(camera_format(&[]), None);
}
#[test]
fn mute_camera_off_and_stop_discard_queued_capture() {
    let e = Engine::create(false);
    e.configure(false, true, 150_000, 1);
    let generation = e.shared.settings().unwrap().generation;
    for kind in [1, 2] {
        e.shared.emit(
            generation,
            DesktopCallEvent::Encoded {
                kind,
                timestamp_us: 0,
                key_frame: true,
                data: vec![1],
            },
        );
    }
    e.configure(true, false, 150_000, 1);
    e.configure(false, true, 150_000, 1);
    assert!(e.poll().is_empty(), "pre-mute media escaped after unmute");
    let generation = e.shared.settings().unwrap().generation;
    e.shared.emit(
        generation,
        DesktopCallEvent::Encoded {
            kind: 1,
            timestamp_us: 0,
            key_frame: false,
            data: vec![1],
        },
    );
    assert_eq!(e.poll().len(), 1);
    for _ in 0..100 {
        e.shared.emit(generation, DesktopCallEvent::RequestKeyFrame);
    }
    assert!(e.shared.output.lock().unwrap().len() <= 24);
    e.shared.fail("Microphone disconnected.");
    assert!(
        e.poll()
            .iter()
            .any(|event| matches!(event, DesktopCallEvent::Error { .. })),
        "a full media queue must not hide device failure"
    );
    e.stop();
    e.shared.emit(0, DesktopCallEvent::Ready);
    assert!(e.poll().is_empty());
}
