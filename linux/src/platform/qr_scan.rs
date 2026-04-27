use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;

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
        .title("Scan QR from image")
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
