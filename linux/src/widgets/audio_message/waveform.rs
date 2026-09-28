use super::*;
use gst::prelude::*;
use std::{
    path::Path,
    time::{Duration, Instant},
};

pub const BARS: usize = 47;

// Decode in a worker, with bounded memory/time and no audio output device.
// A failed/slow analysis leaves a flat seek control; it never blocks playback.
pub fn decode(path: &Path) -> Option<Vec<f32>> {
    gst::init().ok()?;
    let sink = gst::parse::bin_from_description(
        "audioconvert ! audioresample ! audio/x-raw,format=F32LE,channels=1,rate=8000 ! appsink name=peaks sync=false max-buffers=2", true,
    ).ok()?;
    let samples = sink.by_name("peaks")?;
    let player = gst::ElementFactory::make("playbin")
        .property("uri", gtk::gio::File::for_path(path).uri().as_str())
        .property("audio-sink", &sink)
        .build()
        .ok()?;
    player.set_property_from_str("flags", "audio");
    let result = (|| {
        player.set_state(gst::State::Paused).ok()?;
        player.state(gst::ClockTime::from_seconds(2)).0.ok()?;
        let duration = player.query_duration::<gst::ClockTime>()?.seconds_f64();
        if duration <= 0.0 {
            return None;
        }
        player.set_state(gst::State::Playing).ok()?;
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut sums = [0.0_f64; BARS];
        let mut counts = [0u64; BARS];
        let mut frame = 0usize;
        loop {
            if Instant::now() >= deadline {
                return None;
            }
            let sample =
                samples.emit_by_name::<Option<gst::Sample>>("try-pull-sample", &[&100_000_000u64]);
            let Some(sample) = sample else {
                if samples.property::<bool>("eos") {
                    break;
                }
                continue;
            };
            let data = sample.buffer()?.map_readable().ok()?;
            for bytes in data.chunks_exact(4) {
                let value = f32::from_le_bytes(bytes.try_into().ok()?).abs();
                let index =
                    ((frame as f64 / (duration * 8000.0) * BARS as f64) as usize).min(BARS - 1);
                if value.is_finite() {
                    sums[index] += (value as f64).powi(2);
                    counts[index] += 1;
                }
                frame += 1;
            }
        }
        if frame == 0 {
            return None;
        }
        let mut peaks: Vec<f32> = sums
            .iter()
            .zip(counts)
            .map(|(sum, count)| (sum / count.max(1) as f64).sqrt() as f32)
            .collect();
        let max = peaks.iter().copied().fold(0.0_f32, f32::max);
        if max > 0.0001 {
            peaks.iter_mut().for_each(|p| *p /= max);
        }
        Some(peaks)
    })();
    let _ = player.set_state(gst::State::Null);
    result
}

pub fn view(
    owner: &Rc<RefCell<Playback>>,
    progress: &gtk::Scale,
) -> (gtk::Overlay, gtk::DrawingArea) {
    let drawing = gtk::DrawingArea::new();
    drawing.set_content_height(32);
    drawing.set_hexpand(true);
    drawing.set_can_target(false);
    let weak = Rc::downgrade(owner);
    drawing.set_draw_func(move |view, context, width, height| {
        let Some(owner) = weak.upgrade() else {
            return;
        };
        let state = owner.borrow();
        let color = view.color();
        let fraction = if state.duration > 0.0 {
            state.elapsed / state.duration
        } else {
            0.0
        };
        // Match the Scale's 10px thumb travel inset, including at either end.
        let width = (width as f64 - 20.0).max(1.0);
        let step = width / BARS as f64;
        for i in 0..BARS {
            let amplitude = state.peaks.get(i).copied().unwrap_or(0.0) as f64;
            let bar_height = 3.0 + amplitude * 21.0;
            let alpha = if (i as f64 + 0.5) / BARS as f64 <= fraction {
                1.0
            } else {
                0.35
            };
            context.set_source_rgba(
                color.red() as f64,
                color.green() as f64,
                color.blue() as f64,
                alpha,
            );
            context.rectangle(
                10.0 + i as f64 * step,
                (height as f64 - bar_height) / 2.0,
                (step - 1.5).max(1.0),
                bar_height,
            );
            let _ = context.fill();
        }
    });
    progress.add_css_class("audio-waveform");
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(progress));
    overlay.add_overlay(&drawing);
    (overlay, drawing)
}
