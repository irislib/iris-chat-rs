//! Platform consent/capture only; frames still use DesktopCallMedia's H.264 path.
use adw::prelude::*;
use gst::prelude::*;
use gstreamer as gst;
use gtk::{gio, glib};
use iris_chat_core::DesktopCallMedia;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ffi::{c_char, c_int, c_ulong, c_void, CStr, CString},
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    rc::Rc,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};

const DEST: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const SCREEN: &str = "org.freedesktop.portal.ScreenCast";
type Dict = HashMap<String, glib::Variant>;
static TOKEN: AtomicU64 = AtomicU64::new(1);
fn token() -> String {
    format!(
        "iris_{}_{}",
        std::process::id(),
        TOKEN.fetch_add(1, Ordering::Relaxed)
    )
}

#[derive(Default)]
pub(super) struct Selection {
    cancelled: Cell<bool>,
    connection: RefCell<Option<gio::DBusConnection>>,
    session: RefCell<Option<String>>,
    request: RefCell<Option<(String, async_channel::Sender<glib::Variant>)>>,
    dialog: RefCell<Option<adw::AlertDialog>>,
}
impl Selection {
    pub(super) fn cancel(&self) {
        self.cancelled.set(true);
        if let Some(dialog) = self.dialog.borrow_mut().take() {
            dialog.close();
        }
        if let Some((path, sender)) = self.request.borrow_mut().take() {
            self.close(&path, "org.freedesktop.portal.Request");
            let _ = sender.try_send((1u32, Dict::new()).to_variant());
        }
        if let Some(path) = self.session.borrow_mut().take() {
            self.close(&path, "org.freedesktop.portal.Session");
        }
    }
    fn close(&self, path: &str, interface: &str) {
        if let Some(bus) = self.connection.borrow().as_ref() {
            bus.call(
                Some(DEST),
                path,
                interface,
                "Close",
                None,
                None,
                gio::DBusCallFlags::NONE,
                3000,
                gio::Cancellable::NONE,
                |_| {},
            );
        }
    }
}
impl Drop for Selection {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub(super) struct Capture {
    pipeline: gst::Pipeline,
    finished: Arc<AtomicBool>,
    selection: Rc<Selection>,
    subscription: Option<gio::SignalSubscriptionId>,
    // Keep the portal remote alive for the entire pipeline lifetime.
    _remote: Option<OwnedFd>,
}
impl Capture {
    pub(super) fn finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }
    pub(super) fn start(&self, media: Arc<DesktopCallMedia>) -> Result<(), String> {
        let sink = self
            .pipeline
            .by_name("frames")
            .ok_or("Screen capture is unavailable.")?
            .downcast::<gstreamer_app::AppSink>()
            .map_err(|_| "Screen capture is unavailable.")?;
        let finished = self.finished.clone();
        sink.set_callbacks(
            gstreamer_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    if finished.load(Ordering::Acquire) {
                        return Err(gst::FlowError::Eos);
                    }
                    let info = gstreamer_video::VideoInfo::from_caps(
                        sample.caps().ok_or(gst::FlowError::Error)?,
                    )
                    .map_err(|_| gst::FlowError::Error)?;
                    let frame = gstreamer_video::VideoFrameRef::from_buffer_ref_readable(
                        sample.buffer().ok_or(gst::FlowError::Error)?,
                        &info,
                    )
                    .map_err(|_| gst::FlowError::Error)?;
                    use gstreamer_video::prelude::*;
                    let data = frame.plane_data(0).map_err(|_| gst::FlowError::Error)?;
                    let stride = frame.plane_stride()[0] as usize;
                    let row = info.width() as usize * 4;
                    if stride < row || data.len() < stride * info.height() as usize {
                        return Err(gst::FlowError::Error);
                    }
                    let rgba = data
                        .chunks(stride)
                        .take(info.height() as usize)
                        .flat_map(|line| line[..row].iter().copied())
                        .collect();
                    media.submit_video_frame(info.width(), info.height(), rgba);
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );
        self.pipeline
            .set_state(gst::State::Playing)
            .map_err(|_| "Couldn’t start screen sharing.")?;
        let bus = self
            .pipeline
            .bus()
            .ok_or("Couldn’t watch screen sharing.")?;
        let done = self.finished.clone();
        std::thread::spawn(move || {
            while !done.load(Ordering::Acquire) {
                if let Some(message) = bus.timed_pop(gst::ClockTime::from_mseconds(100)) {
                    if matches!(
                        message.view(),
                        gst::MessageView::Error(_) | gst::MessageView::Eos(_)
                    ) {
                        done.store(true, Ordering::Release);
                    }
                }
            }
        });
        Ok(())
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        self.finished.store(true, Ordering::Release);
        let _ = self.pipeline.set_state(gst::State::Null);
        if let Some(id) = self.subscription.take() {
            if let Some(bus) = self.selection.connection.borrow().as_ref() {
                bus.signal_unsubscribe(id);
            }
        }
        self.selection.cancel();
    }
}

async fn request(
    bus: &gio::DBusConnection,
    selection: &Selection,
    method: &str,
    mut args: Vec<glib::Variant>,
    mut options: Dict,
) -> Result<Option<Dict>, String> {
    if selection.cancelled.get() {
        return Ok(None);
    }
    let handle = token();
    options.insert("handle_token".into(), handle.to_variant());
    args.push(options.to_variant());
    let unique = bus
        .unique_name()
        .ok_or("Screen sharing is unavailable.")?
        .trim_start_matches(':')
        .replace('.', "_");
    let path = format!("/org/freedesktop/portal/desktop/request/{unique}/{handle}");
    let (tx, rx) = async_channel::bounded(1);
    let sender = tx.clone();
    let subscription = bus.signal_subscribe(
        Some(DEST),
        Some("org.freedesktop.portal.Request"),
        Some("Response"),
        Some(&path),
        None,
        gio::DBusSignalFlags::NONE,
        move |_, _, _, _, _, response| {
            let _ = sender.try_send(response.clone());
        },
    );
    *selection.request.borrow_mut() = Some((path, tx));
    let result = bus
        .call_future(
            Some(DEST),
            PATH,
            SCREEN,
            method,
            Some(&glib::Variant::tuple_from_iter(args)),
            None,
            gio::DBusCallFlags::NONE,
            10_000,
        )
        .await;
    let response = if result.is_ok() {
        rx.recv()
            .await
            .map_err(|_| "Screen selection was closed.".to_string())
    } else {
        Err("Screen sharing is unavailable in this desktop session.".into())
    };
    bus.signal_unsubscribe(subscription);
    selection.request.borrow_mut().take();
    if selection.cancelled.get() {
        return Ok(None);
    }
    let response = response?;
    let (code, values) = response
        .get::<(u32, Dict)>()
        .ok_or("Invalid screen selection response.")?;
    match code {
        0 => Ok(Some(values)),
        1 => Ok(None),
        _ => Err("Screen sharing was not allowed.".into()),
    }
}

pub(super) async fn choose(
    parent: &gtk::Window,
    selection: Rc<Selection>,
) -> Result<Option<Capture>, String> {
    gst::init().map_err(|_| "Screen sharing is unavailable.")?;
    let bus = gio::bus_get_future(gio::BusType::Session)
        .await
        .map_err(|_| "Couldn’t open the screen picker.")?;
    *selection.connection.borrow_mut() = Some(bus.clone());
    // Detect portal availability before prompting. Never fall back after denial.
    let types = bus
        .call_future(
            Some(DEST),
            PATH,
            "org.freedesktop.DBus.Properties",
            "Get",
            Some(&(SCREEN, "AvailableSourceTypes").to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            3000,
        )
        .await;
    if types.is_err()
        && gtk::prelude::WidgetExt::display(parent)
            .type_()
            .name()
            .contains("X11")
    {
        return choose_x11(parent, selection).await;
    }
    let types = types
        .map_err(|_| "Screen sharing needs the desktop screen-sharing portal.")?
        .child_value(0)
        .as_variant()
        .and_then(|v| v.get::<u32>())
        .unwrap_or(1)
        & 3;
    let mut options = Dict::new();
    options.insert("session_handle_token".into(), token().to_variant());
    let Some(result) = request(&bus, &selection, "CreateSession", vec![], options).await? else {
        return Ok(None);
    };
    let session = result
        .get("session_handle")
        .and_then(|v| v.get::<String>())
        .ok_or("Couldn’t open the screen picker.")?;
    *selection.session.borrow_mut() = Some(session.clone());
    let session_path = glib::variant::ObjectPath::try_from(session.as_str())
        .map_err(|_| "Invalid screen session.")?;
    let mut options = Dict::new();
    options.insert("types".into(), types.to_variant());
    options.insert("multiple".into(), false.to_variant());
    if request(
        &bus,
        &selection,
        "SelectSources",
        vec![session_path.to_variant()],
        options,
    )
    .await?
    .is_none()
    {
        return Ok(None);
    }
    // An empty parent identifier is supported by the portal on Wayland and X11;
    // the desktop owns and identifies the consent dialog independently.
    let Some(result) = request(
        &bus,
        &selection,
        "Start",
        vec![session_path.to_variant(), "".to_variant()],
        Dict::new(),
    )
    .await?
    else {
        return Ok(None);
    };
    let streams = result
        .get("streams")
        .and_then(|v| v.get::<Vec<(u32, Dict)>>())
        .ok_or("No screen was selected.")?;
    let (node, properties) = streams.first().ok_or("No screen was selected.")?;
    let (reply, fds) = bus
        .call_with_unix_fd_list_future(
            Some(DEST),
            PATH,
            SCREEN,
            "OpenPipeWireRemote",
            Some(&(session_path, Dict::new()).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            3000,
            None::<&gio::UnixFDList>,
        )
        .await
        .map_err(|_| "Couldn’t open the selected screen.")?;
    let index = reply
        .child_value(0)
        .get::<glib::variant::Handle>()
        .ok_or("Invalid screen connection.")?;
    let raw = fds
        .ok_or("Missing screen connection.")?
        .get(index.0)
        .map_err(|_| "Couldn’t open the selected screen.")?;
    let remote = unsafe { OwnedFd::from_raw_fd(raw) };
    let source = gst::ElementFactory::make("pipewiresrc")
        .property("fd", remote.as_raw_fd())
        .build()
        .map_err(|_| "Install the GStreamer PipeWire plugin to share your screen.")?;
    // Version 6 portals provide a stable PipeWire serial; older portals use node ID.
    if let Some(serial) = properties
        .get("pipewire-serial")
        .and_then(|v| v.get::<u64>())
    {
        if source.find_property("target-object").is_some() {
            source.set_property("target-object", serial.to_string());
        } else {
            source.set_property("path", node.to_string());
        }
    } else {
        source.set_property("path", node.to_string());
    }
    let mut capture = pipeline(source, selection.clone())?;
    let done = capture.finished.clone();
    capture.subscription = Some(bus.signal_subscribe(
        Some(DEST),
        Some("org.freedesktop.portal.Session"),
        Some("Closed"),
        Some(&session),
        None,
        gio::DBusSignalFlags::NONE,
        move |_, _, _, _, _, _| {
            done.store(true, Ordering::Release);
        },
    ));
    capture._remote = Some(remote);
    if selection.cancelled.get() {
        return Ok(None);
    }
    Ok(Some(capture))
}

fn pipeline(source: gst::Element, selection: Rc<Selection>) -> Result<Capture, String> {
    let pipeline = gst::Pipeline::new();
    let tail = gst::parse::bin_from_description("queue max-size-buffers=1 leaky=downstream ! videoconvert ! videoscale ! videorate drop-only=true ! video/x-raw,format=RGBA,width=[1,1920],height=[1,1080],framerate=15/1,pixel-aspect-ratio=1/1 ! appsink name=frames max-buffers=1 drop=true sync=false", true).map_err(|_| "Screen sharing needs the GStreamer video plugins.")?;
    pipeline
        .add_many([&source, tail.upcast_ref()])
        .map_err(|_| "Couldn’t prepare screen sharing.")?;
    source
        .link(&tail)
        .map_err(|_| "Couldn’t prepare screen sharing.")?;
    Ok(Capture {
        pipeline,
        selection,
        finished: Arc::new(AtomicBool::new(false)),
        subscription: None,
        _remote: None,
    })
}

async fn choose_x11(
    parent: &gtk::Window,
    selection: Rc<Selection>,
) -> Result<Option<Capture>, String> {
    let choices = x11_choices(parent);
    if choices.is_empty() {
        return Err("No screens are available to share.".into());
    }
    let dialog = adw::AlertDialog::builder().heading("Share screen").build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("share", "Share");
    dialog.set_response_appearance("share", adw::ResponseAppearance::Suggested);
    dialog.set_close_response("cancel");
    let dropdown = gtk::DropDown::from_strings(
        &choices
            .iter()
            .map(|(_, name)| name.as_str())
            .collect::<Vec<_>>(),
    );
    dropdown.set_margin_top(18);
    dropdown.set_margin_bottom(18);
    dropdown.set_margin_start(18);
    dropdown.set_margin_end(18);
    dialog.set_extra_child(Some(&dropdown));
    *selection.dialog.borrow_mut() = Some(dialog.clone());
    let response = dialog.clone().choose_future(parent).await;
    selection.dialog.borrow_mut().take();
    dialog.close();
    if response != "share" || selection.cancelled.get() {
        return Ok(None);
    }
    let (xid, _) = choices
        .get(dropdown.selected() as usize)
        .ok_or("No screen was selected.")?;
    let source = gst::ElementFactory::make("ximagesrc")
        .property("xid", *xid as u64)
        .property("use-damage", false)
        .property("show-pointer", true)
        .build()
        .map_err(|_| "Install the GStreamer X11 capture plugin to share your screen.")?;
    pipeline(source, selection).map(Some)
}

fn x11_choices(parent: &gtk::Window) -> Vec<(c_ulong, String)> {
    use glib::translate::ToGlibPtr;
    // GTK traps X11 errors on its own connection, including a window closing
    // during enumeration. Never install a process-wide Xlib error handler.
    unsafe {
        let display = gtk::prelude::WidgetExt::display(parent);
        let gdk_display = display.to_glib_none().0;
        gdk_x11_display_error_trap_push(gdk_display);
        let xdisplay = gdk_x11_display_get_xdisplay(gdk_display);
        let root = XDefaultRootWindow(xdisplay);
        let mut result = vec![(root, "Entire screen".into())];
        let atom = XInternAtom(
            xdisplay,
            CString::new("_NET_CLIENT_LIST").unwrap().as_ptr(),
            1,
        );
        let mut windows = Vec::new();
        if atom != 0 {
            let mut actual_type = 0;
            let mut format = 0;
            let mut count = 0;
            let mut remaining = 0;
            let mut data = std::ptr::null_mut();
            if XGetWindowProperty(
                xdisplay,
                root,
                atom,
                0,
                4096,
                0,
                33,
                &mut actual_type,
                &mut format,
                &mut count,
                &mut remaining,
                &mut data,
            ) == 0
                && !data.is_null()
            {
                if format == 32 && actual_type == 33 {
                    windows.extend_from_slice(std::slice::from_raw_parts(
                        data.cast::<c_ulong>(),
                        count.min(4096) as usize,
                    ));
                }
                XFree(data.cast());
            }
        }
        // No window manager (e.g. Xvfb): direct top-level children are sufficient.
        if windows.is_empty() {
            let mut returned_root = 0;
            let mut parent = 0;
            let mut children = std::ptr::null_mut();
            let mut count = 0;
            if XQueryTree(
                xdisplay,
                root,
                &mut returned_root,
                &mut parent,
                &mut children,
                &mut count,
            ) != 0
                && !children.is_null()
            {
                windows.extend_from_slice(std::slice::from_raw_parts(
                    children,
                    count.min(4096) as usize,
                ));
                XFree(children.cast());
            }
        }
        for window in windows {
            let mut name = std::ptr::null_mut();
            if XFetchName(xdisplay, window, &mut name) != 0 && !name.is_null() {
                let title = CStr::from_ptr(name).to_string_lossy().trim().to_string();
                XFree(name.cast());
                if !title.is_empty() {
                    result.push((window, title));
                }
            }
        }
        gdk_x11_display_error_trap_pop_ignored(gdk_display);
        result
    }
}
#[link(name = "gtk-4")]
unsafe extern "C" {
    fn gdk_x11_display_get_xdisplay(display: *mut gtk::gdk::ffi::GdkDisplay) -> *mut c_void;
    fn gdk_x11_display_error_trap_push(display: *mut gtk::gdk::ffi::GdkDisplay);
    fn gdk_x11_display_error_trap_pop_ignored(display: *mut gtk::gdk::ffi::GdkDisplay);
}
#[link(name = "X11")]
unsafe extern "C" {
    fn XDefaultRootWindow(display: *mut c_void) -> c_ulong;
    fn XInternAtom(display: *mut c_void, name: *const c_char, only_if_exists: c_int) -> c_ulong;
    fn XGetWindowProperty(
        display: *mut c_void,
        window: c_ulong,
        property: c_ulong,
        offset: std::ffi::c_long,
        length: std::ffi::c_long,
        delete: c_int,
        requested_type: c_ulong,
        actual_type: *mut c_ulong,
        format: *mut c_int,
        count: *mut c_ulong,
        remaining: *mut c_ulong,
        data: *mut *mut u8,
    ) -> c_int;
    fn XQueryTree(
        display: *mut c_void,
        window: c_ulong,
        root: *mut c_ulong,
        parent: *mut c_ulong,
        children: *mut *mut c_ulong,
        count: *mut u32,
    ) -> c_int;
    fn XFetchName(display: *mut c_void, window: c_ulong, name: *mut *mut c_char) -> c_int;
    fn XFree(data: *mut c_void) -> c_int;
}

#[cfg(feature = "ui-tests")]
pub(super) fn verify_capture_ui() {
    use std::time::{Duration, Instant};
    fn pump(mut ready: impl FnMut() -> bool) {
        let context = glib::MainContext::default();
        let deadline = Instant::now() + Duration::from_secs(8);
        while !ready() {
            while context.pending() {
                context.iteration(false);
            }
            assert!(Instant::now() < deadline, "screen capture timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    let window = gtk::Window::builder()
        .title("Screen sharing test")
        .default_width(400)
        .default_height(220)
        .build();
    window.set_child(Some(&gtk::Label::new(Some(
        "Only this selected surface is shared",
    ))));
    window.present();
    gst::init().unwrap();
    assert!(
        gtk::prelude::WidgetExt::display(&window)
            .type_()
            .name()
            .contains("X11"),
        "fixture requires Xvfb/X11"
    );
    for accept in [false, true] {
        let selection = Rc::new(Selection::default());
        let result = Rc::new(RefCell::new(None));
        let completion = result.clone();
        let choosing = selection.clone();
        let parent = window.clone();
        glib::MainContext::default().spawn_local(async move {
            *completion.borrow_mut() = Some(choose_x11(&parent, choosing).await);
        });
        pump(|| selection.dialog.borrow().is_some());
        let dialog = selection.dialog.borrow().clone().unwrap();
        if accept {
            if let Some(path) = std::env::var_os("IRIS_SCREEN_PICKER_SNAPSHOT") {
                let mut node = None;
                pump(|| {
                    let paintable = gtk::WidgetPaintable::new(Some(&dialog));
                    let snapshot = gtk::Snapshot::new();
                    paintable.snapshot(&snapshot, dialog.width() as f64, dialog.height() as f64);
                    node = snapshot.to_node();
                    node.is_some() && dialog.width() > 0
                });
                dialog
                    .native()
                    .expect("screen picker surface")
                    .renderer()
                    .expect("screen picker renderer")
                    .render_texture(node.as_ref().unwrap(), None)
                    .save_to_png(path)
                    .expect("screen picker screenshot");
            }
            fn button(widget: &gtk::Widget) -> Option<gtk::Button> {
                if let Ok(button) = widget.clone().downcast::<gtk::Button>() {
                    if button.label().as_deref() == Some("Share") {
                        return Some(button);
                    }
                }
                let mut child = widget.first_child();
                while let Some(current) = child {
                    if let Some(found) = button(&current) {
                        return Some(found);
                    }
                    child = current.next_sibling();
                }
                None
            }
            button(dialog.upcast_ref())
                .expect("Share button")
                .emit_clicked();
        } else {
            selection.cancel();
        }
        pump(|| result.borrow().is_some());
        let captured = result.borrow_mut().take().unwrap().unwrap();
        if !accept {
            assert!(captured.is_none());
            continue;
        }
        let capture = captured.expect("accepted screen selection");
        let media = DesktopCallMedia::new();
        media.set_external_video(true);
        media.configure(true, true, 500_000, 1);
        capture.start(media.clone()).unwrap();
        let mut encoded = false;
        pump(|| {
            for event in media.poll() {
                if matches!(
                    event,
                    iris_chat_core::DesktopCallEvent::Encoded { kind: 2, .. }
                ) {
                    encoded = true;
                }
            }
            encoded
        });
        assert!(!capture.finished());
        let stopped = capture.finished.clone();
        drop(capture);
        media.stop();
        assert!(stopped.load(Ordering::Acquire));
        assert!(media.poll().is_empty());
    }
    window.destroy();
    eprintln!("PASS native X11 screen picker cancel, capture to H264 and stop cleanup");
}
