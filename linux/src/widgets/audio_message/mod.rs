use adw::prelude::*;
use gstreamer as gst;
use gtk::glib;
use iris_chat_core::MessageAttachmentSnapshot;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::{Rc, Weak},
};

pub const CSS: &str = ".chat-audio-player scale.audio-waveform { padding: 0 10px; min-height: 32px; } .chat-audio-player scale.audio-waveform trough, .chat-audio-player scale.audio-waveform highlight { background: transparent; box-shadow: none; border: none; } .chat-audio-player scale.audio-waveform slider { background: currentColor; min-width: 6px; min-height: 6px; padding: 0; margin: 0; box-shadow: none; border: none; }";

mod playback;
mod waveform;
use playback::Playback;

thread_local! {
    static PLAYERS: RefCell<HashMap<String, Rc<RefCell<Playback>>>> = RefCell::new(HashMap::new());
    static ACTIVE: RefCell<Weak<RefCell<Playback>>> = RefCell::new(Weak::new());
    static CALL_ACTIVE: Cell<bool> = const { Cell::new(false) };
}

pub fn set_call_active(active: bool) {
    CALL_ACTIVE.set(active);
    if active {
        ACTIVE.with(|player| {
            if let Some(player) = player.borrow().upgrade() {
                player.borrow_mut().pause();
            }
        });
    }
}

pub fn is_audio(attachment: &MessageAttachmentSnapshot) -> bool {
    attachment.is_audio
        || matches!(
            attachment
                .filename
                .rsplit('.')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase()
                .as_str(),
            "aac" | "aiff" | "flac" | "m4a" | "mp3" | "ogg" | "opus" | "wav" | "wma"
        )
}

pub fn widget(message_id: &str, attachment: &MessageAttachmentSnapshot) -> gtk::Box {
    let key = format!("{}:{}", message_id, attachment.htree_url);
    let owner = PLAYERS.with(|players| {
        players
            .borrow_mut()
            .entry(key.clone())
            .or_insert_with(|| Rc::new(RefCell::new(Playback::new(key, attachment.clone()))))
            .clone()
    });
    let root = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    root.add_css_class("chat-audio-player");
    root.set_size_request(244, -1);
    root.set_margin_top(4);
    root.set_margin_bottom(4);
    let play = gtk::Button::from_icon_name("media-playback-start-symbolic");
    play.add_css_class("circular");
    play.set_size_request(44, 44);
    play.set_valign(gtk::Align::Center);
    root.append(&play);
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    column.set_hexpand(true);
    let progress = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.1);
    progress.set_draw_value(false);
    progress.set_hexpand(true);
    progress.set_sensitive(false);
    progress.update_property(&[gtk::accessible::Property::Label("Audio position")]);
    let (waveform, drawing) = waveform::view(&owner, &progress);
    column.append(&waveform);
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let time = gtk::Label::new(Some("Audio"));
    time.add_css_class("dim-label");
    time.add_css_class("caption");
    time.set_hexpand(true);
    time.set_xalign(0.0);
    footer.append(&time);
    let speed = gtk::Button::with_label("1×");
    speed.add_css_class("flat");
    speed.add_css_class("caption");
    speed.update_property(&[gtk::accessible::Property::Label("Playback speed")]);
    footer.append(&speed);
    column.append(&footer);
    root.append(&column);
    let updating = Rc::new(Cell::new(false));
    owner.borrow_mut().controls.push(Controls {
        play: play.downgrade(),
        progress: progress.downgrade(),
        waveform: drawing.downgrade(),
        time: time.downgrade(),
        speed: speed.downgrade(),
        updating: updating.clone(),
    });
    let state = owner.clone();
    play.connect_clicked(move |_| Playback::toggle(&state));
    let state = owner.clone();
    speed.connect_clicked(move |_| state.borrow_mut().cycle_rate());
    let state = owner.clone();
    progress.connect_value_changed(move |scale| {
        if !updating.get() {
            state.borrow_mut().seek(scale.value());
        }
    });
    let state = owner.clone();
    root.connect_map(move |_| {
        state.borrow_mut().views += 1;
        let key = state.borrow().key.clone();
        PLAYERS.with(|players| {
            players.borrow_mut().insert(key, state.clone());
        });
    });
    let state = owner.clone();
    root.connect_unmap(move |_| {
        state.borrow_mut().views -= 1;
        retire_when_unused(state.clone());
    });
    retire_when_unused(owner.clone());
    owner.borrow_mut().refresh();
    root
}

fn retire_when_unused(owner: Rc<RefCell<Playback>>) {
    // Timeline replacement for receipts/new messages reattaches before idle.
    glib::idle_add_local_once(move || {
        if owner.borrow().views == 0 {
            owner.borrow_mut().stop();
            let key = owner.borrow().key.clone();
            PLAYERS.with(|players| {
                let mut players = players.borrow_mut();
                if players
                    .get(&key)
                    .is_some_and(|current| Rc::ptr_eq(current, &owner))
                {
                    players.remove(&key);
                }
            });
        }
    });
}

struct Controls {
    play: glib::WeakRef<gtk::Button>,
    progress: glib::WeakRef<gtk::Scale>,
    waveform: glib::WeakRef<gtk::DrawingArea>,
    time: glib::WeakRef<gtk::Label>,
    speed: glib::WeakRef<gtk::Button>,
    updating: Rc<Cell<bool>>,
}

impl Playback {
    fn refresh(&mut self) {
        self.controls.retain(|ui| {
            let (Some(play), Some(progress), Some(time), Some(speed)) = (
                ui.play.upgrade(),
                ui.progress.upgrade(),
                ui.time.upgrade(),
                ui.speed.upgrade(),
            ) else {
                return false;
            };
            ui.updating.set(true);
            let (icon, action) = if self.loading {
                ("process-stop-symbolic", "Cancel loading")
            } else if self.error.is_some() {
                ("view-refresh-symbolic", "Retry audio")
            } else if self.playing {
                ("media-playback-pause-symbolic", "Pause audio")
            } else {
                ("media-playback-start-symbolic", "Play audio")
            };
            play.set_icon_name(icon);
            play.set_tooltip_text(Some(action));
            play.update_property(&[gtk::accessible::Property::Label(action)]);
            progress.set_range(0.0, self.duration.max(1.0));
            progress.set_value(self.elapsed);
            progress.set_sensitive(self.duration > 0.0 && !self.loading && self.error.is_none());
            time.set_text(self.error.unwrap_or(&if self.duration > 0.0 {
                format!("{} / {}", timestamp(self.elapsed), timestamp(self.duration))
            } else {
                "Audio".into()
            }));
            speed.set_label(&format!("{}×", self.rate));
            speed.set_tooltip_text(Some(&format!("Playback speed: {}×", self.rate)));
            if let Some(waveform) = ui.waveform.upgrade() {
                waveform.queue_draw();
            }
            ui.updating.set(false);
            true
        });
    }
}

fn timestamp(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[cfg(feature = "ui-tests")]
#[allow(dead_code)]
pub mod tests;
