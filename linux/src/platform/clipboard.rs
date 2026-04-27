use adw::prelude::*;
use gtk::gdk;
use gtk::gio;

pub fn copy(text: &str) {
    if let Some(display) = gdk::Display::default() {
        display.clipboard().set_text(text);
    }
}

pub fn paste<F: FnOnce(String) + 'static>(callback: F) {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let clipboard = display.clipboard();
    clipboard.read_text_async(gio::Cancellable::NONE, move |result| {
        let value = match result {
            Ok(Some(text)) => text.to_string(),
            _ => String::new(),
        };
        callback(value);
    });
}
