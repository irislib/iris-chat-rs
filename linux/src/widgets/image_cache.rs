use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};
use std::thread;

use gtk::gdk;
use gtk::glib;

static BYTES_CACHE: LazyLock<Mutex<HashMap<String, Vec<u8>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

static IN_FLIGHT: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

pub fn fetch_into_picture(picture: &gtk::Picture, url: &str) {
    let pic = picture.clone();
    fetch_bytes(url, move |bytes| {
        let glib_bytes = glib::Bytes::from(bytes);
        if let Ok(texture) = gdk::Texture::from_bytes(&glib_bytes) {
            pic.set_paintable(Some(&texture));
        }
    });
}

pub fn fetch_into_avatar(avatar: &adw::Avatar, url: &str) {
    let av = avatar.clone();
    fetch_bytes(url, move |bytes| {
        let glib_bytes = glib::Bytes::from(bytes);
        if let Ok(texture) = gdk::Texture::from_bytes(&glib_bytes) {
            av.set_custom_image(Some(&texture));
        }
    });
}

fn fetch_bytes<F>(url: &str, on_loaded: F)
where
    F: FnOnce(&[u8]) + 'static,
{
    if url.is_empty() {
        return;
    }

    if let Some(bytes) = BYTES_CACHE.lock().unwrap().get(url).cloned() {
        on_loaded(&bytes);
        return;
    }

    let url_owned = url.to_string();
    {
        let mut in_flight = IN_FLIGHT.lock().unwrap();
        if !in_flight.insert(url_owned.clone()) {
            return;
        }
    }

    let (tx, rx) = async_channel::bounded::<Option<Vec<u8>>>(1);
    let url_for_thread = url_owned.clone();
    thread::spawn(move || {
        let bytes = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .ok()
            .and_then(|c| c.get(&url_for_thread).send().ok())
            .and_then(|resp| resp.bytes().ok())
            .map(|b| b.to_vec());
        let _ = tx.send_blocking(bytes);
    });

    let url_for_main = url_owned;
    glib::MainContext::default().spawn_local(async move {
        if let Ok(Some(bytes)) = rx.recv().await {
            BYTES_CACHE
                .lock()
                .unwrap()
                .insert(url_for_main.clone(), bytes.clone());
            on_loaded(&bytes);
        }
        IN_FLIGHT.lock().unwrap().remove(&url_for_main);
    });
}
