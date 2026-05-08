use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};
use std::thread;

use gtk::gdk;
use gtk::glib;

static BYTES_CACHE: LazyLock<Mutex<HashMap<String, Vec<u8>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

static IN_FLIGHT: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

type ImageLoadCallback = Box<dyn FnOnce(&[u8]) + 'static>;

thread_local! {
    static WAITERS: RefCell<HashMap<String, Vec<ImageLoadCallback>>> = RefCell::new(HashMap::new());
    static TEXTURES: RefCell<HashMap<String, gdk::Texture>> = RefCell::new(HashMap::new());
}

pub fn fetch_into_picture(picture: &gtk::Picture, url: &str) {
    if let Some(texture) = cached_texture(url) {
        picture.set_paintable(Some(&texture));
        return;
    }

    let pic = picture.clone();
    let url_owned = url.to_string();
    fetch_bytes(url, move |bytes| {
        let glib_bytes = glib::Bytes::from(bytes);
        if let Ok(texture) = gdk::Texture::from_bytes(&glib_bytes) {
            cache_texture(&url_owned, &texture);
            pic.set_paintable(Some(&texture));
        }
    });
}

pub fn fetch_into_avatar(avatar: &adw::Avatar, url: &str) {
    if let Some(texture) = cached_texture(url) {
        avatar.set_custom_image(Some(&texture));
        return;
    }

    let av = avatar.clone();
    let url_owned = url.to_string();
    fetch_bytes(url, move |bytes| {
        let glib_bytes = glib::Bytes::from(bytes);
        if let Ok(texture) = gdk::Texture::from_bytes(&glib_bytes) {
            cache_texture(&url_owned, &texture);
            av.set_custom_image(Some(&texture));
        }
    });
}

fn cached_texture(url: &str) -> Option<gdk::Texture> {
    TEXTURES.with(|textures| textures.borrow().get(url).cloned())
}

fn cache_texture(url: &str, texture: &gdk::Texture) {
    TEXTURES.with(|textures| {
        textures
            .borrow_mut()
            .insert(url.to_string(), texture.clone());
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
    let should_start = {
        let mut in_flight = IN_FLIGHT.lock().unwrap();
        in_flight.insert(url_owned.clone())
    };

    WAITERS.with(|waiters| {
        waiters
            .borrow_mut()
            .entry(url_owned.clone())
            .or_default()
            .push(Box::new(on_loaded));
    });

    if !should_start {
        return;
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
            let callbacks = WAITERS.with(|waiters| {
                waiters
                    .borrow_mut()
                    .remove(&url_for_main)
                    .unwrap_or_default()
            });
            for callback in callbacks {
                callback(&bytes);
            }
        } else {
            WAITERS.with(|waiters| {
                waiters.borrow_mut().remove(&url_for_main);
            });
        }
        IN_FLIGHT.lock().unwrap().remove(&url_for_main);
    });
}
