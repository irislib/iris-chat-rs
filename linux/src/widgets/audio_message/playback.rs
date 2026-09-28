use super::*;
use base64::Engine;
use gst::prelude::*;
use std::io::Write;
use std::time::Duration;

pub(super) struct Playback {
    pub key: String,
    pub attachment: MessageAttachmentSnapshot,
    pub pipeline: Option<gst::Element>,
    file: Option<tempfile::TempPath>,
    bus_watch: Option<gst::bus::BusWatchGuard>,
    timer: Option<glib::SourceId>,
    pub loading: bool,
    pub playing: bool,
    pub elapsed: f64,
    pub duration: f64,
    pub rate: f64,
    pub peaks: Vec<f32>,
    pub error: Option<&'static str>,
    pub views: usize,
    pub controls: Vec<Controls>,
    generation: u64,
}

impl Playback {
    pub fn new(key: String, attachment: MessageAttachmentSnapshot) -> Self {
        Self {
            key,
            attachment,
            pipeline: None,
            file: None,
            bus_watch: None,
            timer: None,
            loading: false,
            playing: false,
            elapsed: 0.0,
            duration: 0.0,
            rate: 1.0,
            peaks: Vec::new(),
            error: None,
            views: 0,
            controls: Vec::new(),
            generation: 0,
        }
    }

    pub fn toggle(owner: &Rc<RefCell<Self>>) {
        if owner.borrow().playing || owner.borrow().loading {
            owner.borrow_mut().pause();
            return;
        }
        if CALL_ACTIVE.get() {
            let mut state = owner.borrow_mut();
            state.error = Some("Finish your call to play audio.");
            state.refresh();
            return;
        }
        ACTIVE.with(|active| {
            if let Some(previous) = active.borrow().upgrade().filter(|p| !Rc::ptr_eq(p, owner)) {
                previous.borrow_mut().pause();
            }
            *active.borrow_mut() = Rc::downgrade(owner);
        });
        let mut state = owner.borrow_mut();
        state.error = None;
        if state.pipeline.is_some() && state.duration > 0.0 {
            if state.elapsed >= state.duration - 0.05 {
                state.seek(0.0);
            }
            drop(state);
            Self::start(owner);
            return;
        }
        state.generation += 1;
        let generation = state.generation;
        state.loading = true;
        state.refresh();
        let attachment = state.attachment.clone();
        drop(state);
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let result = load_attachment(&attachment).map(|file| {
                let peaks = waveform::decode(&file).unwrap_or_default();
                (file, peaks)
            });
            let _ = tx.send_blocking(result);
        });
        let weak = Rc::downgrade(owner);
        glib::spawn_future_local(async move {
            let result = rx.recv().await;
            let Some(owner) = weak.upgrade() else {
                return;
            };
            if owner.borrow().generation != generation {
                return;
            }
            match result {
                Ok(Ok((file, peaks))) => {
                    owner.borrow_mut().peaks = peaks;
                    if Self::prepare(&owner, file).is_err() {
                        owner.borrow_mut().fail();
                    }
                }
                _ => owner.borrow_mut().fail(),
            }
        });
    }

    fn prepare(owner: &Rc<RefCell<Self>>, file: tempfile::TempPath) -> Result<(), String> {
        gst::init().map_err(|e| e.to_string())?;
        let tempo = gst::ElementFactory::make("scaletempo")
            .build()
            .map_err(|e| e.to_string())?;
        let uri = gtk::gio::File::for_path(&file).uri();
        let pipeline = gst::ElementFactory::make("playbin")
            .property("uri", uri.as_str())
            .property("audio-filter", &tempo)
            .build()
            .map_err(|e| e.to_string())?;
        pipeline.set_property_from_str("flags", "audio");
        #[cfg(feature = "ui-tests")]
        if std::env::var("IRIS_UI_TEST_AUDIO_SINK").as_deref() == Ok("fakesink") {
            let sink = gst::ElementFactory::make("fakesink")
                .property("sync", true)
                .build()
                .map_err(|e| e.to_string())?;
            pipeline.set_property("audio-sink", &sink);
        }
        let weak = Rc::downgrade(owner);
        let bus = pipeline.bus().ok_or("Audio bus unavailable")?;
        let watch = bus
            .add_watch_local(move |_, message| {
                let Some(owner) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                match message.view() {
                    gst::MessageView::AsyncDone(_) if owner.borrow().loading => {
                        {
                            let mut state = owner.borrow_mut();
                            state.duration = state
                                .pipeline
                                .as_ref()
                                .and_then(|p| p.query_duration::<gst::ClockTime>())
                                .map(|t| t.nseconds() as f64 / 1e9)
                                .unwrap_or(0.0);
                            state.loading = false;
                            state.seek(0.0);
                        }
                        Self::start(&owner);
                    }
                    gst::MessageView::Eos(_) => {
                        let mut state = owner.borrow_mut();
                        state.pause();
                        state.elapsed = state.duration;
                        state.refresh();
                    }
                    gst::MessageView::Error(_) => owner.borrow_mut().fail(),
                    _ => {}
                }
                glib::ControlFlow::Continue
            })
            .map_err(|e| e.to_string())?;
        pipeline
            .set_state(gst::State::Paused)
            .map_err(|e| e.to_string())?;
        let mut state = owner.borrow_mut();
        state.file = Some(file);
        state.pipeline = Some(pipeline);
        state.bus_watch = Some(watch);
        Ok(())
    }

    fn start(owner: &Rc<RefCell<Self>>) {
        if CALL_ACTIVE.get() {
            owner.borrow_mut().pause();
            return;
        }
        let mut state = owner.borrow_mut();
        let Some(pipeline) = &state.pipeline else {
            return;
        };
        if pipeline.set_state(gst::State::Playing).is_err() {
            state.fail();
            return;
        }
        state.playing = true;
        state.loading = false;
        state.refresh();
        let weak = Rc::downgrade(owner);
        state.timer = Some(glib::timeout_add_local(
            Duration::from_millis(100),
            move || {
                let Some(owner) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                let mut state = owner.borrow_mut();
                if let Some(position) = state
                    .pipeline
                    .as_ref()
                    .and_then(|p| p.query_position::<gst::ClockTime>())
                {
                    state.elapsed = position.nseconds() as f64 / 1e9;
                }
                state.refresh();
                glib::ControlFlow::Continue
            },
        ));
    }

    pub fn pause(&mut self) {
        self.generation += 1;
        if let Some(timer) = self.timer.take() {
            timer.remove();
        }
        if self.loading {
            self.close();
        } else if let Some(pipeline) = &self.pipeline {
            let _ = pipeline.set_state(gst::State::Paused);
        }
        self.loading = false;
        self.playing = false;
        self.refresh();
    }

    pub fn seek(&mut self, seconds: f64) {
        if !seconds.is_finite() || self.duration <= 0.0 {
            return;
        }
        let Some(pipeline) = &self.pipeline else {
            return;
        };
        let seconds = seconds.clamp(0.0, self.duration);
        if pipeline
            .seek(
                self.rate,
                gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                gst::SeekType::Set,
                gst::ClockTime::from_nseconds((seconds * 1e9) as u64),
                gst::SeekType::None,
                gst::ClockTime::NONE,
            )
            .is_err()
        {
            self.fail();
            return;
        }
        self.elapsed = seconds;
        self.refresh();
    }

    pub fn cycle_rate(&mut self) {
        self.rate = if self.rate == 1.0 {
            1.5
        } else if self.rate == 1.5 {
            2.0
        } else if self.rate == 2.0 {
            0.5
        } else {
            1.0
        };
        self.seek(self.elapsed);
        self.refresh();
    }

    pub fn stop(&mut self) {
        self.pause();
        self.close();
    }

    fn fail(&mut self) {
        self.pause();
        self.close();
        self.error = Some("Couldn't play audio. Try again.");
        self.refresh();
    }

    fn close(&mut self) {
        // Drop the watch before closing the pipeline so stale errors cannot
        // reach a later retry. TempPath deletes decrypted bytes after close.
        self.bus_watch = None;
        if let Some(pipeline) = self.pipeline.take() {
            let _ = pipeline.set_state(gst::State::Null);
        }
        self.file = None;
    }
}

impl Drop for Playback {
    fn drop(&mut self) {
        if let Some(timer) = self.timer.take() {
            timer.remove();
        }
        self.close();
    }
}

fn load_attachment(attachment: &MessageAttachmentSnapshot) -> Result<tempfile::TempPath, String> {
    #[cfg(feature = "ui-tests")]
    let encoded = if attachment.nhash == "audio-test" {
        let path = std::env::var("IRIS_UI_TEST_AUDIO_FILE").map_err(|e| e.to_string())?;
        Some(
            base64::engine::general_purpose::STANDARD
                .encode(std::fs::read(path).map_err(|e| e.to_string())?),
        )
    } else {
        iris_chat_core::download_hashtree_attachment(attachment.nhash.clone()).data_base64
    };
    #[cfg(not(feature = "ui-tests"))]
    let encoded =
        iris_chat_core::download_hashtree_attachment(attachment.nhash.clone()).data_base64;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded.ok_or("Audio unavailable")?)
        .map_err(|e| e.to_string())?;
    if bytes.is_empty() {
        return Err("Audio unavailable".into());
    }
    let mut file = tempfile::Builder::new()
        .prefix("iris-audio-")
        .tempfile()
        .map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    Ok(file.into_temp_path())
}
