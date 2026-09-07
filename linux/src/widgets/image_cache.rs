use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{LazyLock, Mutex};
use std::thread;

use gtk::gdk;
use gtk::glib;
use gtk::prelude::ObjectExt;
use iris_chat_core::{image_load_urls, PreferencesSnapshot};

static BYTES_CACHE: LazyLock<Mutex<HashMap<String, Vec<u8>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

static IN_FLIGHT: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

type TextureLoadCallback = Box<dyn FnOnce(gdk::Texture) + 'static>;

type ImageLoadCallback = Box<dyn FnOnce(Option<&[u8]>) + 'static>;

thread_local! {
    static WAITERS: RefCell<HashMap<String, Vec<ImageLoadCallback>>> = RefCell::new(HashMap::new());
    static TEXTURES: RefCell<HashMap<String, gdk::Texture>> = RefCell::new(HashMap::new());
}

pub fn fetch_into_picture(
    picture: &gtk::Picture,
    original: &str,
    urls: &[String],
    preferences: &PreferencesSnapshot,
) {
    let picture = picture.downgrade();
    let current = picture.clone();
    fetch_texture(
        urls.to_vec(),
        original_redirect_source(original, preferences),
        Box::new(move |texture| {
            if let Some(picture) = picture.upgrade() {
                picture.set_paintable(Some(&texture));
            }
        }),
        Rc::new(move || current.upgrade().is_some()),
    );
}

pub fn prefetch(original: &str, urls: &[String], preferences: &PreferencesSnapshot) {
    fetch_texture(
        urls.to_vec(),
        original_redirect_source(original, preferences),
        Box::new(|_| {}),
        Rc::new(|| true),
    );
}

pub fn fetch_proxied_into_avatar(
    avatar: &adw::Avatar,
    url: &str,
    preferences: &PreferencesSnapshot,
    size: u32,
) {
    let urls = image_load_urls(
        url.to_string(),
        preferences.clone(),
        Some(size),
        Some(size),
        true,
    );
    let avatar = avatar.downgrade();
    let current = avatar.clone();
    fetch_texture(
        urls,
        original_redirect_source(url, preferences),
        Box::new(move |texture| {
            if let Some(avatar) = avatar.upgrade() {
                avatar.set_custom_image(Some(&texture));
            }
        }),
        Rc::new(move || current.upgrade().is_some()),
    );
}

fn original_redirect_source(original: &str, preferences: &PreferencesSnapshot) -> Option<String> {
    (!preferences.image_proxy_enabled || preferences.image_proxy_fallback_enabled)
        .then(|| original.to_string())
}

fn fetch_texture(
    urls: Vec<String>,
    original: Option<String>,
    on_loaded: TextureLoadCallback,
    is_current: Rc<dyn Fn() -> bool>,
) {
    if !is_current() {
        return;
    }
    let mut candidates = urls.into_iter();
    let Some(url) = candidates.next() else {
        return;
    };
    if let Some(texture) = cached_texture(&url) {
        on_loaded(texture);
        return;
    }
    let url_owned = url.clone();
    fetch_bytes(
        &url,
        original.as_deref() == Some(url.as_str()),
        move |bytes| {
            if !is_current() {
                return;
            }
            let texture =
                bytes.and_then(|bytes| gdk::Texture::from_bytes(&glib::Bytes::from(bytes)).ok());
            if let Some(texture) = texture {
                cache_texture(&url_owned, &texture);
                on_loaded(texture);
            } else {
                BYTES_CACHE.lock().unwrap().remove(&url_owned);
                fetch_texture(candidates.collect(), original, on_loaded, is_current);
            }
        },
    );
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

fn fetch_bytes<F>(url: &str, allow_cross_origin_redirects: bool, on_loaded: F)
where
    F: FnOnce(Option<&[u8]>) + 'static,
{
    if url.is_empty() {
        on_loaded(None);
        return;
    }

    let cached_bytes = BYTES_CACHE.lock().unwrap().get(url).cloned();
    if let Some(bytes) = cached_bytes {
        on_loaded(Some(&bytes));
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
        let origin = reqwest::Url::parse(&url_for_thread)
            .ok()
            .map(|url| url.origin());
        let redirects = if allow_cross_origin_redirects {
            reqwest::redirect::Policy::limited(10)
        } else {
            reqwest::redirect::Policy::custom(move |attempt| {
                if attempt.previous().len() >= 10 {
                    attempt.error("too many image redirects")
                } else if Some(attempt.url().origin()) == origin {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            })
        };
        let bytes = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .redirect(redirects)
            .build()
            .ok()
            .and_then(|c| c.get(&url_for_thread).send().ok())
            .and_then(|resp| resp.error_for_status().ok())
            .and_then(|resp| resp.bytes().ok())
            .map(|b| b.to_vec());
        let _ = tx.send_blocking(bytes);
    });

    let url_for_main = url_owned;
    glib::MainContext::default().spawn_local(async move {
        let bytes = rx.recv().await.ok().flatten();
        if let Some(bytes) = &bytes {
            BYTES_CACHE
                .lock()
                .unwrap()
                .insert(url_for_main.clone(), bytes.clone());
        }
        IN_FLIGHT.lock().unwrap().remove(&url_for_main);
        let callbacks = WAITERS.with(|waiters| {
            waiters
                .borrow_mut()
                .remove(&url_for_main)
                .unwrap_or_default()
        });
        for callback in callbacks {
            callback(bytes.as_deref());
        }
    });
}

#[cfg(test)]
#[path = "image_cache_tests.rs"]
mod tests;
