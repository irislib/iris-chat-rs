use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;

#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(target_os = "linux")]
use std::sync::Arc;
#[cfg(target_os = "linux")]
use std::thread;
#[cfg(target_os = "linux")]
use std::time::Duration;
#[cfg(target_os = "linux")]
use gtk::{gdk, glib};
#[cfg(target_os = "linux")]
use image::ImageReader;
#[cfg(target_os = "linux")]
use v4l::{
    buffer::Type,
    io::traits::CaptureStream,
    prelude::*,
    video::Capture,
    Device, FourCC,
};

pub fn decode_image_file(path: &Path) -> Option<String> {
    let img = image::open(path).ok()?;
    let luma = img.to_luma8();
    let mut prepared = rqrr::PreparedImage::prepare(luma);
    prepared
        .detect_grids()
        .into_iter()
        .find_map(|grid| grid.decode().ok().map(|(_, content)| content))
}

pub fn pick_and_decode<F: Fn(String) + 'static>(parent: Option<&gtk::Window>, on_result: F) {
    let dialog = gtk::FileDialog::builder()
        .title("Scan code from image")
        .build();
    let on_result = Rc::new(on_result);
    dialog.open(parent, gtk::gio::Cancellable::NONE, move |result| {
        let Ok(file) = result else {
            return;
        };
        let Some(path) = file.path() else {
            return;
        };
        if let Some(text) = decode_image_file(&path) {
            (on_result)(text);
        }
    });
}

#[cfg(target_os = "linux")]
pub fn camera_available() -> bool {
    Device::new(0).is_ok()
}

#[cfg(not(target_os = "linux"))]
#[allow(dead_code)]
pub fn camera_available() -> bool {
    false
}

#[cfg(not(target_os = "linux"))]
pub fn open_scanner<F: Fn(String) + 'static>(parent: Option<&gtk::Window>, on_result: F) {
    pick_and_decode(parent, on_result);
}

#[cfg(target_os = "linux")]
pub fn open_scanner<F: Fn(String) + 'static>(parent: Option<&gtk::Window>, on_result: F) {
    if !camera_available() {
        // Fall back to image-file scanning when no camera is reachable
        // (e.g. dev container without device passthrough).
        pick_and_decode(parent, on_result);
        return;
    }

    let dialog = adw::Dialog::builder()
        .title("Scan code")
        .content_width(360)
        .build();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(20);
    content.set_margin_bottom(20);
    content.set_margin_start(20);
    content.set_margin_end(20);

    let picture = gtk::Picture::new();
    picture.set_size_request(320, 240);
    picture.set_can_shrink(true);
    picture.set_content_fit(gtk::ContentFit::Cover);
    picture.add_css_class("card");
    content.append(&picture);

    let status = gtk::Label::new(Some("Point the camera at a code"));
    status.add_css_class("dim-label");
    content.append(&status);

    let pick_image = gtk::Button::with_label("Pick image instead");
    pick_image.add_css_class("flat");
    content.append(&pick_image);

    dialog.set_child(Some(&content));

    let stop_flag = Arc::new(AtomicBool::new(false));
    let (frame_tx, frame_rx) = async_channel::bounded::<CameraFrame>(2);
    let (text_tx, text_rx) = async_channel::bounded::<String>(1);

    let stop_for_thread = stop_flag.clone();
    thread::spawn(move || {
        capture_loop(stop_for_thread, frame_tx, text_tx);
    });

    let pic_for_frames = picture.clone();
    glib::MainContext::default().spawn_local(async move {
        while let Ok(frame) = frame_rx.recv().await {
            let bytes = glib::Bytes::from(&frame.rgb);
            let texture = gdk::MemoryTexture::new(
                frame.width as i32,
                frame.height as i32,
                gdk::MemoryFormat::R8g8b8,
                &bytes,
                frame.width as usize * 3,
            );
            pic_for_frames.set_paintable(Some(&texture));
        }
    });

    let on_result = Rc::new(on_result);
    let dialog_for_match = dialog.clone();
    let stop_for_match = stop_flag.clone();
    let or = on_result.clone();
    glib::MainContext::default().spawn_local(async move {
        if let Ok(text) = text_rx.recv().await {
            (or)(text);
            stop_for_match.store(true, Ordering::SeqCst);
            dialog_for_match.close();
        }
    });

    let parent_for_pick = parent.cloned();
    let or_pick = on_result.clone();
    let stop_for_pick = stop_flag.clone();
    let dialog_for_pick = dialog.clone();
    pick_image.connect_clicked(move |_| {
        let stop = stop_for_pick.clone();
        let or = or_pick.clone();
        let dialog = dialog_for_pick.clone();
        let parent = parent_for_pick.clone();
        pick_and_decode(parent.as_ref(), move |text| {
            stop.store(true, Ordering::SeqCst);
            (or)(text);
            dialog.close();
        });
    });

    let stop_for_close = stop_flag.clone();
    dialog.connect_closed(move |_| {
        stop_for_close.store(true, Ordering::SeqCst);
    });

    dialog.present(parent);
}

#[cfg(target_os = "linux")]
struct CameraFrame {
    rgb: Vec<u8>,
    width: u32,
    height: u32,
}

#[cfg(target_os = "linux")]
fn capture_loop(
    stop: Arc<AtomicBool>,
    frame_tx: async_channel::Sender<CameraFrame>,
    text_tx: async_channel::Sender<String>,
) {
    let Ok(dev) = Device::new(0) else {
        return;
    };

    let mut format = match dev.format() {
        Ok(f) => f,
        Err(_) => return,
    };
    format.fourcc = FourCC::new(b"MJPG");
    let _ = dev.set_format(&format);

    let format = match dev.format() {
        Ok(f) => f,
        Err(_) => return,
    };
    let is_mjpeg = &format.fourcc.repr == b"MJPG";

    let Ok(mut stream) = MmapStream::with_buffers(&dev, Type::VideoCapture, 4) else {
        return;
    };

    while !stop.load(Ordering::SeqCst) {
        let Ok((buf, _meta)) = stream.next() else {
            thread::sleep(Duration::from_millis(40));
            continue;
        };

        let Some(rgb_image) = decode_frame(buf, &format, is_mjpeg) else {
            continue;
        };

        let luma = image::imageops::grayscale(&rgb_image);
        let mut prepared = rqrr::PreparedImage::prepare(luma);
        if let Some((_, text)) = prepared
            .detect_grids()
            .into_iter()
            .find_map(|grid| grid.decode().ok())
        {
            let _ = text_tx.send_blocking(text);
            return;
        }

        let frame = CameraFrame {
            width: rgb_image.width(),
            height: rgb_image.height(),
            rgb: rgb_image.into_raw(),
        };
        if frame_tx.send_blocking(frame).is_err() {
            return;
        }
    }
}

#[cfg(target_os = "linux")]
fn decode_frame(
    buffer: &[u8],
    format: &v4l::Format,
    is_mjpeg: bool,
) -> Option<image::RgbImage> {
    if is_mjpeg {
        let cursor = std::io::Cursor::new(buffer);
        let dynamic = ImageReader::with_format(cursor, image::ImageFormat::Jpeg)
            .decode()
            .ok()?;
        Some(dynamic.to_rgb8())
    } else {
        // YUYV422 fallback: convert pairs to grayscale RGB approximation.
        let width = format.width as usize;
        let height = format.height as usize;
        if buffer.len() < width * height * 2 {
            return None;
        }
        let mut rgb = vec![0u8; width * height * 3];
        for i in 0..(width * height) {
            let y = buffer[i * 2] as u8;
            rgb[i * 3] = y;
            rgb[i * 3 + 1] = y;
            rgb[i * 3 + 2] = y;
        }
        image::RgbImage::from_raw(format.width, format.height, rgb)
    }
}
