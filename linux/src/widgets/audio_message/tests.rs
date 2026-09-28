use super::*;
use std::time::{Duration, Instant};

fn pump_until(mut ready: impl FnMut() -> bool) {
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        while context.pending() {
            context.iteration(false);
        }
        if ready() {
            return;
        }
        assert!(Instant::now() < deadline, "Audio playback timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}

pub fn run() {
    adw::init().expect("GTK display required");
    adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
    let css = gtk::CssProvider::new();
    css.load_from_string(&format!("{}\n{}", ".bubble-in, .bubble-out { padding: 7px 12px; border-radius: 18px; color: white; } .bubble-in { background: #3A3A3A; } .bubble-out { background: #702ACE; }", CSS));
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().unwrap(),
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        file.path(),
        include_bytes!("../../../../test-fixtures/voice-message.m4a"),
    )
    .unwrap();
    std::env::set_var("IRIS_UI_TEST_AUDIO_FILE", file.path());
    std::env::set_var("IRIS_UI_TEST_AUDIO_SINK", "fakesink");
    let attachment = MessageAttachmentSnapshot {
        nhash: "audio-test".into(),
        filename: "Voice message.M4A".into(),
        filename_encoded: "Voice%20message.M4A".into(),
        htree_url: "htree://audio-test/Voice%20message.M4A".into(),
        is_image: false,
        is_video: false,
        is_audio: false,
    };
    assert!(
        is_audio(&attachment),
        "Old attachments must work without the audio flag"
    );
    assert!(!is_audio(&MessageAttachmentSnapshot {
        filename: "document.pdf".into(),
        ..attachment.clone()
    }));
    let window = gtk::Window::builder()
        .title("Voice messages")
        .default_width(560)
        .default_height(260)
        .build();
    let column = gtk::Box::new(gtk::Orientation::Vertical, 24);
    column.set_margin_top(24);
    column.set_margin_bottom(24);
    column.set_margin_start(24);
    column.set_margin_end(24);
    window.set_child(Some(&column));
    let incoming = widget("incoming", &attachment);
    incoming.add_css_class("bubble-in");
    incoming.set_halign(gtk::Align::Start);
    let outgoing = widget("outgoing", &attachment);
    outgoing.add_css_class("bubble-out");
    outgoing.set_halign(gtk::Align::End);
    column.append(&incoming);
    column.append(&outgoing);
    window.present();
    pump_until(|| incoming.is_mapped());
    let first =
        PLAYERS.with(|p| p.borrow()["incoming:htree://audio-test/Voice%20message.M4A"].clone());
    let second =
        PLAYERS.with(|p| p.borrow()["outgoing:htree://audio-test/Voice%20message.M4A"].clone());
    assert!(
        first.borrow().pipeline.is_none(),
        "Don't download before Play"
    );
    let play = incoming
        .first_child()
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap();
    play.emit_clicked();
    pump_until(|| first.borrow().playing && first.borrow().elapsed > 0.1);
    assert!((first.borrow().duration - 6.0).abs() < 0.2);
    assert_eq!(first.borrow().peaks.len(), 47);
    assert!(
        first.borrow().peaks[18..22].iter().all(|p| *p < 0.02),
        "Real silence stays flat"
    );
    assert!(
        first.borrow().peaks[..14].iter().any(|p| *p > 0.5),
        "Real audio has peaks"
    );
    play.emit_clicked();
    assert!(!first.borrow().playing);
    first.borrow_mut().seek(3.0);
    assert!((first.borrow().elapsed - 3.0).abs() < 0.1);
    for rate in [1.5, 2.0, 0.5, 1.0] {
        first.borrow_mut().cycle_rate();
        assert_eq!(first.borrow().rate, rate);
        assert!(!first.borrow().playing);
    }
    play.emit_clicked();
    pump_until(|| first.borrow().playing);
    // Production row replacement must retain the playing session.
    column.remove(&incoming);
    let replacement = widget("incoming", &attachment);
    replacement.set_halign(gtk::Align::Start);
    replacement.add_css_class("bubble-in");
    column.prepend(&replacement);
    pump_until(|| replacement.is_mapped());
    assert!(first.borrow().playing);
    Playback::toggle(&second);
    pump_until(|| second.borrow().playing);
    assert!(!first.borrow().playing, "Only one message may play");
    set_call_active(true);
    assert!(!second.borrow().playing);
    Playback::toggle(&first);
    assert!(first.borrow().error.is_some());
    assert!(!first.borrow().playing);
    set_call_active(false);
    Playback::toggle(&second);
    pump_until(|| second.borrow().playing);
    second.borrow_mut().seek(5.8);
    pump_until(|| !second.borrow().playing);
    assert!((second.borrow().elapsed - second.borrow().duration).abs() < 0.1);
    Playback::toggle(&second);
    pump_until(|| second.borrow().playing && second.borrow().elapsed < 1.0);
    second.borrow_mut().pause();
    first.borrow_mut().error = None;
    first.borrow_mut().cycle_rate();
    first.borrow_mut().refresh();
    if let Ok(path) = std::env::var("IRIS_AUDIO_SCREENSHOT") {
        let paintable = gtk::WidgetPaintable::new(Some(&column));
        let mut node = None;
        pump_until(|| {
            let snapshot = gtk::Snapshot::new();
            paintable.snapshot(&snapshot, column.width() as f64, column.height() as f64);
            node = snapshot.to_node();
            node.is_some()
        });
        let node = node.unwrap();
        let renderer = window.renderer().unwrap();
        let texture = renderer.render_texture(&node, None);
        texture.save_to_png(path).unwrap();
    }
    window.close();
    pump_until(|| PLAYERS.with(|p| p.borrow().is_empty()));
    assert!(!first.borrow().playing && !second.borrow().playing);
    println!("PASS: GTK M4A decode, progress, pause, seek, speed, replay, single playback, calls, row replacement and cleanup");
}
