use crate::app_manager::AppManager;
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use iris_chat_core::{
    AppAction, CallSnapshot, DesktopCallEvent, DesktopCallMedia, DesktopCallTone,
};
use std::{cell::RefCell, rc::Rc, sync::Arc, time::Duration};

pub struct Calls {
    manager: Rc<AppManager>,
    window: gtk::Window,
    title: gtk::Label,
    status: gtk::Label,
    remote: gtk::Picture,
    local: gtk::Picture,
    answer: gtk::Button,
    voice: gtk::Button,
    end: gtk::Button,
    mute: gtk::Button,
    camera: gtk::Button,
    ending: Option<String>,
    last_ring: std::time::Instant,
    call: Option<CallSnapshot>,
    media: Option<Arc<DesktopCallMedia>>,
    tone: Option<Arc<DesktopCallTone>>,
}
impl Calls {
    pub fn new(parent: &adw::ApplicationWindow, manager: Rc<AppManager>) -> Rc<RefCell<Self>> {
        let window = gtk::Window::builder()
            .title("Call")
            .transient_for(parent)
            .default_width(580)
            .default_height(460)
            .build();
        let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
        column.set_margin_top(20);
        column.set_margin_bottom(20);
        column.set_margin_start(20);
        column.set_margin_end(20);
        let title = gtk::Label::new(None);
        title.add_css_class("title-2");
        column.append(&title);
        let status = gtk::Label::new(None);
        column.append(&status);
        let remote = gtk::Picture::new();
        remote.set_vexpand(true);
        remote.set_hexpand(true);
        remote.set_can_shrink(true);
        column.append(&remote);
        let local = gtk::Picture::new();
        local.set_size_request(160, 90);
        local.set_halign(gtk::Align::End);
        local.set_can_shrink(true);
        column.append(&local);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.set_halign(gtk::Align::Center);
        let answer = gtk::Button::with_label("Answer");
        answer.add_css_class("suggested-action");
        let voice = gtk::Button::with_label("Voice only");
        let end = gtk::Button::with_label("End call");
        end.add_css_class("destructive-action");
        let mute = gtk::Button::with_label("Mute");
        let camera = gtk::Button::with_label("Camera off");
        for b in [&answer, &voice, &mute, &camera, &end] {
            row.append(b);
        }
        column.append(&row);
        window.set_child(Some(&column));
        let calls = Rc::new(RefCell::new(Self {
            manager,
            window: window.clone(),
            title,
            status,
            remote,
            local,
            answer: answer.clone(),
            voice: voice.clone(),
            end: end.clone(),
            mute: mute.clone(),
            camera: camera.clone(),
            ending: None,
            last_ring: std::time::Instant::now() - Duration::from_secs(5),
            call: None,
            media: None,
            tone: None,
        }));
        for (button, action) in [(answer, 0), (voice, 1), (end, 2), (mute, 3), (camera, 4)] {
            let weak = Rc::downgrade(&calls);
            button.connect_clicked(move |_| {
                if let Some(c) = weak.upgrade() {
                    c.borrow_mut().action(action);
                }
            });
        }
        let weak = Rc::downgrade(&calls);
        window.connect_close_request(move |_| {
            if let Some(c) = weak.upgrade() {
                c.borrow_mut().action(2);
            }
            glib::Propagation::Stop
        });
        let weak = Rc::downgrade(&calls);
        parent.connect_destroy(move |_| {
            if let Some(c) = weak.upgrade() {
                c.borrow_mut().shutdown();
            }
        });
        let weak = Rc::downgrade(&calls);
        glib::timeout_add_local(Duration::from_millis(15), move || {
            let Some(c) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            c.borrow_mut().poll();
            glib::ControlFlow::Continue
        });
        calls
    }
    fn action(&mut self, action: u8) {
        let Some(call) = self.call.clone() else {
            return;
        };
        let action = match action {
            0 => AppAction::AnswerCall {
                call_id: call.call_id,
            },
            1 => AppAction::AnswerCallWithVoice {
                call_id: call.call_id,
            },
            2 => {
                self.ending = Some(call.call_id.clone());
                self.stop();
                self.window.set_visible(false);
                AppAction::EndCall {
                    call_id: call.call_id,
                }
            }
            3 => AppAction::SetCallMuted { muted: !call.muted },
            _ => AppAction::SetCallVideoEnabled {
                enabled: !call.video,
            },
        };
        self.manager.dispatch(action);
    }
    pub fn sync(&mut self, call: Option<CallSnapshot>) {
        let changed = self.call.as_ref().map(|c| &c.call_id) != call.as_ref().map(|c| &c.call_id);
        if changed {
            self.ending = None;
            self.stop();
            self.local.set_paintable(gdk::Paintable::NONE);
            self.remote.set_paintable(gdk::Paintable::NONE);
        }
        self.call = call;
        let Some(call) = self.call.as_ref() else {
            self.stop();
            self.window.set_visible(false);
            return;
        };
        if self.ending.as_deref() == Some(&call.call_id) {
            self.stop();
            self.window.set_visible(false);
            return;
        }
        if let Some(column) = self.window.child() {
            column.set_valign(
                if call.phase == "connected" && (call.video || call.remote_video) {
                    gtk::Align::Fill
                } else {
                    gtk::Align::Center
                },
            );
        }
        let incoming = call.phase == "incoming";
        let connected = call.phase == "connected";
        self.title.set_label(&call.peer_name);
        self.status.set_label(if call.phase == "ended" {
            call.end_reason.as_deref().unwrap_or("Call ended")
        } else if incoming {
            if call.video_capable {
                "Incoming video call"
            } else {
                "Incoming voice call"
            }
        } else if call.phase == "ringing" {
            "Ringing…"
        } else if !connected {
            "Calling…"
        } else if !call.media_connected {
            "Connecting…"
        } else if call.remote_muted {
            "Microphone muted"
        } else {
            "Connected"
        });
        self.answer.set_visible(incoming);
        self.voice.set_visible(incoming && call.video_capable);
        self.end.set_label(if call.phase == "ended" {
            "Done"
        } else if incoming {
            "Decline"
        } else {
            "End call"
        });
        self.mute.set_visible(connected);
        self.mute
            .set_label(if call.muted { "Unmute" } else { "Mute" });
        self.camera.set_visible(connected && call.video_capable);
        self.camera.set_label(if call.video {
            "Camera off"
        } else {
            "Camera on"
        });
        self.local.set_visible(connected && call.video);
        self.remote.set_visible(connected && call.remote_video);
        if call.outgoing && matches!(call.phase.as_str(), "outgoing" | "ringing") {
            self.tone
                .get_or_insert_with(|| DesktopCallTone::new(call.phase == "ringing"))
                .set_ringing(call.phase == "ringing");
        } else if let Some(tone) = self.tone.take() {
            tone.stop();
        }
        if connected {
            let media = self.media.get_or_insert_with(DesktopCallMedia::new);
            media.configure(
                call.muted,
                call.video,
                call.target_bitrate_bps,
                call.key_frame_generation,
            );
        } else if let Some(media) = self.media.take() {
            media.stop();
        }
        if changed {
            self.window.present();
            if incoming {
                crate::platform::notifications::notify(
                    "iris-call",
                    &self.title.text(),
                    "Incoming call",
                );
            }
        }
        if !incoming {
            if let Some(app) = gio::Application::default() {
                app.withdraw_notification("iris-call");
            }
        }
    }
    pub fn receive(
        &self,
        id: &str,
        kind: u8,
        sequence: u32,
        timestamp: u64,
        key: bool,
        data: Vec<u8>,
    ) {
        if self
            .call
            .as_ref()
            .is_some_and(|c| c.call_id == id && c.phase == "connected")
        {
            if let Some(media) = &self.media {
                media.receive(kind, sequence, timestamp, key, data);
            }
        }
    }
    fn poll(&mut self) {
        if self
            .call
            .as_ref()
            .is_some_and(|c| c.phase == "incoming" && self.ending.as_deref() != Some(&c.call_id))
            && self.last_ring.elapsed() >= Duration::from_secs(3)
        {
            self.window.error_bell();
            self.last_ring = std::time::Instant::now();
        }
        let Some(media) = &self.media else { return };
        let events = media.poll();
        let Some(call) = self.call.clone() else {
            return;
        };
        for event in events {
            match event {
                DesktopCallEvent::Encoded {
                    kind,
                    timestamp_us,
                    key_frame,
                    data,
                } => self.manager.dispatch(AppAction::SendCallMedia {
                    call_id: call.call_id.clone(),
                    kind,
                    timestamp_us,
                    key_frame,
                    data,
                }),
                DesktopCallEvent::Ready => {
                    self.manager.dispatch(AppAction::SetCallMediaConnected {
                        call_id: call.call_id.clone(),
                        connected: true,
                    })
                }
                DesktopCallEvent::RequestKeyFrame => {
                    self.manager.dispatch(AppAction::RequestCallKeyFrame {
                        call_id: call.call_id.clone(),
                    })
                }
                DesktopCallEvent::Video {
                    local,
                    width,
                    height,
                    rgba,
                } => {
                    let bytes = glib::Bytes::from_owned(rgba);
                    let texture = gdk::MemoryTexture::new(
                        width as i32,
                        height as i32,
                        gdk::MemoryFormat::R8g8b8a8,
                        &bytes,
                        width as usize * 4,
                    );
                    (if local { &self.local } else { &self.remote }).set_paintable(Some(&texture));
                }
                DesktopCallEvent::Error { message } => {
                    self.action(2);
                    let dialog = adw::AlertDialog::builder()
                        .heading("Call ended")
                        .body(&message)
                        .build();
                    dialog.add_response("close", "Close");
                    dialog.present(self.window.transient_for().as_ref());
                    break;
                }
            }
        }
    }
    fn stop(&mut self) {
        if let Some(tone) = self.tone.take() {
            tone.stop();
        }
        if let Some(media) = self.media.take() {
            media.stop();
        }
        if let Some(app) = gio::Application::default() {
            app.withdraw_notification("iris-call");
        }
    }
    fn shutdown(&mut self) {
        self.action(2);
        self.stop();
        self.window.destroy();
    }
}

#[cfg(feature = "ui-tests")]
pub fn verify_ui(manager: Rc<AppManager>) {
    let parent = adw::ApplicationWindow::builder().title("Call test").build();
    let calls = Calls::new(&parent, manager);
    let mut call = CallSnapshot {
        outgoing: false,
        target_bitrate_bps: 150_000,
        key_frame_generation: 0,
        media_connected: false,
        max_bitrate_bps: 1_200_000,
        call_id: "ui-call".into(),
        chat_id: "peer".into(),
        peer_name: "Alex".into(),
        phase: "incoming".into(),
        video: true,
        video_capable: true,
        muted: false,
        remote_video: true,
        remote_muted: false,
        started_at_secs: 0,
        connected_at_secs: None,
        end_reason: None,
    };
    calls.borrow_mut().sync(Some(call.clone()));
    assert!(
        calls.borrow().media.is_none(),
        "incoming calls must not open microphone or camera"
    );
    assert!(calls.borrow().answer.is_visible());
    assert!(calls.borrow().voice.is_visible());
    assert!(!calls.borrow().camera.is_visible());
    if let Some(path) = std::env::var_os("IRIS_CALL_UI_SNAPSHOT") {
        let context = glib::MainContext::default();
        let deadline = std::time::Instant::now() + Duration::from_millis(300);
        while std::time::Instant::now() < deadline {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let window = &calls.borrow().window;
        let paintable = gtk::WidgetPaintable::new(Some(window));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
        let node = snapshot.to_node().expect("call UI render node");
        let texture = window
            .renderer()
            .expect("call renderer")
            .render_texture(&node, None);
        texture.save_to_png(path).expect("call UI screenshot");
    }
    call.phase = "outgoing".into();
    calls.borrow_mut().sync(Some(call.clone()));
    assert_eq!(calls.borrow().status.text(), "Calling…");
    assert!(
        calls.borrow().media.is_none(),
        "unanswered calls must not capture"
    );
    assert!(!calls.borrow().answer.is_visible());
    call.phase = "ended".into();
    call.end_reason = Some("Call declined".into());
    calls.borrow_mut().sync(Some(call));
    assert_eq!(calls.borrow().status.text(), "Call declined");
    calls.borrow_mut().action(2);
    assert!(!calls.borrow().window.is_visible());
    calls.borrow_mut().sync(None);
    assert!(calls.borrow().media.is_none());
    calls.borrow_mut().shutdown();
    parent.destroy();
    println!("Desktop call UI: ringing, outgoing, declined, and device privacy passed");
}
