use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const PNG: &[u8] = include_bytes!("../../resources/iris-chat-16.png");

struct ImageServer {
    url: String,
    requests: Arc<AtomicUsize>,
    stopped: Arc<AtomicBool>,
    response: Arc<Mutex<(u16, &'static [u8])>>,
    redirect: Arc<Mutex<Option<String>>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ImageServer {
    fn new(status: u16, body: &'static [u8]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let stopped = Arc::new(AtomicBool::new(false));
        let response = Arc::new(Mutex::new((status, body)));
        let current_response = response.clone();
        let redirect = Arc::new(Mutex::new(None::<String>));
        let redirect_to = redirect.clone();
        let request_count = requests.clone();
        let stop = stopped.clone();
        let worker = thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                if let Ok((mut stream, _)) = listener.accept() {
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut request = [0; 4096];
                    stream.read(&mut request).unwrap();
                    request_count.fetch_add(1, Ordering::SeqCst);
                    let (mut status, mut body) = *current_response.lock().unwrap();
                    let mut location = redirect_to.lock().unwrap().clone();
                    if String::from_utf8_lossy(&request).contains(" /redirected ") {
                        status = 200;
                        body = PNG;
                        location = None;
                    }
                    let location = location
                        .map(|url| format!("Location: {url}\r\n"))
                        .unwrap_or_default();
                    write!(
                        stream,
                        "HTTP/1.1 {status} Test\r\n{location}Content-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .unwrap();
                    stream.write_all(body).unwrap();
                } else {
                    thread::sleep(Duration::from_millis(1));
                }
            }
        });
        Self {
            url,
            requests,
            stopped,
            response,
            redirect,
            worker: Some(worker),
        }
    }

    fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

impl Drop for ImageServer {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        let result = self.worker.take().unwrap().join();
        if !thread::panicking() {
            result.unwrap();
        }
    }
}

fn load(urls: Vec<String>, original: &str) -> bool {
    load_with_redirect_source(urls, Some(original.to_string()))
}

fn load_with_redirect_source(urls: Vec<String>, original: Option<String>) -> bool {
    let loaded = Rc::new(RefCell::new(false));
    let result = loaded.clone();
    fetch_texture(
        urls,
        original,
        Box::new(move |_| *result.borrow_mut() = true),
        Rc::new(|| true),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let context = glib::MainContext::default();
    while !IN_FLIGHT.lock().unwrap().is_empty() {
        assert!(Instant::now() < deadline, "image request did not complete");
        while context.iteration(false) {}
        thread::sleep(Duration::from_millis(1));
    }
    let result = *loaded.borrow();
    result
}

#[test]
fn image_fallback_requires_opt_in_and_a_failed_proxy_load() {
    let context = glib::MainContext::default();
    let _guard = context.acquire().unwrap();
    let original = ImageServer::new(200, PNG);
    for (status, body) in [(503, PNG), (200, b"not an image".as_slice())] {
        let proxy = ImageServer::new(status, body);
        let mut preferences = PreferencesSnapshot {
            image_proxy_url: proxy.url.clone(),
            ..PreferencesSnapshot::default()
        };
        let urls = image_load_urls(original.url.clone(), preferences.clone(), None, None, false);
        let original_requests = original.requests();
        assert!(!load(urls, &original.url));
        assert_eq!(original.requests(), original_requests);

        preferences.image_proxy_fallback_enabled = true;
        let urls = image_load_urls(original.url.clone(), preferences, None, None, false);
        assert!(load(urls.clone(), &original.url));
        assert_eq!(proxy.requests(), 2);
        assert!(original.requests() > 0);

        // Invalid decoded bytes must not pin a temporary proxy failure in cache.
        *proxy.response.lock().unwrap() = (200, PNG);
        assert!(load(urls.clone(), &original.url));
        assert_eq!(proxy.requests(), 3);
        assert!(cached_texture(&urls[0]).is_some());
    }

    let proxy = ImageServer::new(200, PNG);
    let preferences = PreferencesSnapshot {
        image_proxy_url: proxy.url.clone(),
        image_proxy_fallback_enabled: true,
        ..PreferencesSnapshot::default()
    };
    let original_requests = original.requests();
    assert!(load(
        image_load_urls(original.url.clone(), preferences, None, None, false),
        &original.url
    ));
    assert_eq!(proxy.requests(), 1);
    assert_eq!(original.requests(), original_requests);

    let redirect_target = ImageServer::new(200, PNG);
    let proxy = ImageServer::new(302, b"");
    *proxy.redirect.lock().unwrap() = Some(redirect_target.url.clone());
    let mut preferences = PreferencesSnapshot {
        image_proxy_url: proxy.url.clone(),
        ..PreferencesSnapshot::default()
    };
    assert!(!load(
        image_load_urls(
            redirect_target.url.clone(),
            preferences.clone(),
            None,
            None,
            false
        ),
        &redirect_target.url,
    ));
    assert_eq!(
        redirect_target.requests(),
        0,
        "proxy redirect exposed the original host"
    );
    preferences.image_proxy_fallback_enabled = true;
    assert!(load(
        image_load_urls(redirect_target.url.clone(), preferences, None, None, false),
        &redirect_target.url,
    ));
    assert_eq!(redirect_target.requests(), 1);

    let already_proxied_target = ImageServer::new(200, PNG);
    let already_proxied = ImageServer::new(302, b"");
    *already_proxied.redirect.lock().unwrap() = Some(already_proxied_target.url.clone());
    let preferences = PreferencesSnapshot {
        image_proxy_url: already_proxied.url.clone(),
        ..PreferencesSnapshot::default()
    };
    let urls = image_load_urls(
        already_proxied.url.clone(),
        preferences.clone(),
        None,
        None,
        false,
    );
    assert!(!load_with_redirect_source(
        urls,
        original_redirect_source(&already_proxied.url, &preferences),
    ));
    assert_eq!(already_proxied_target.requests(), 0);

    let same_origin = ImageServer::new(302, b"");
    *same_origin.redirect.lock().unwrap() = Some("/redirected".to_string());
    assert!(load(vec![same_origin.url.clone()], &original.url));
    assert_eq!(same_origin.requests(), 2);

    let direct_redirect = ImageServer::new(302, b"");
    *direct_redirect.redirect.lock().unwrap() = Some(original.url.clone());
    assert!(load(
        vec![direct_redirect.url.clone()],
        &direct_redirect.url
    ));
    assert_eq!(direct_redirect.requests(), 1);

    let unavailable = TcpListener::bind("127.0.0.1:0").unwrap();
    let unavailable_url = format!("http://{}", unavailable.local_addr().unwrap());
    drop(unavailable);
    assert!(load(
        vec![unavailable_url, original.url.clone()],
        &original.url
    ));
}
