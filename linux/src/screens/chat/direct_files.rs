use std::rc::Rc;

use adw::prelude::*;
use iris_chat_core::{AppAction, DirectFileTransferSnapshot, DirectFileTransferStatus};

use crate::app_manager::AppManager;

pub(super) fn card(
    chat_id: &str,
    transfer: &DirectFileTransferSnapshot,
    manager: &Rc<AppManager>,
) -> gtk::Box {
    use DirectFileTransferStatus as Status;
    let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
    column.set_widget_name(&format!("chatDirectTransfer-{}", transfer.id));
    let title = gtk::Label::new(Some("Direct files"));
    title.set_xalign(0.0);
    title.add_css_class("heading");
    column.append(&title);
    for file in &transfer.files {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let label = gtk::Label::new(Some(&format!(
            "{} · {}",
            file.filename,
            size(file.size_bytes)
        )));
        label.set_xalign(0.0);
        label.set_wrap(true);
        label.set_max_width_chars(32);
        label.set_hexpand(true);
        row.append(&label);
        if transfer.status == Status::Completed {
            if let Some(path) = file.local_path.as_ref() {
                let open = gtk::Button::with_label("Open");
                open.set_widget_name(&format!("chatDirectTransferOpen-{}", transfer.id));
                let path = path.clone();
                open.connect_clicked(move |_| {
                    let file = gtk::gio::File::for_path(&path);
                    let _ = gtk::gio::AppInfo::launch_default_for_uri(
                        file.uri().as_str(),
                        gtk::gio::AppLaunchContext::NONE,
                    );
                });
                row.append(&open);
            }
        }
        column.append(&row);
    }
    let status = match transfer.status {
        Status::Offered if transfer.is_sender => "Waiting for acceptance",
        Status::Offered => "Ready to receive",
        Status::Connecting => "Connecting…",
        Status::Transferring => "Sending…",
        Status::Completed => "Complete",
        Status::Declined => "Declined",
        Status::Cancelled => "Cancelled",
        Status::Failed => "Couldn’t send files",
        Status::Unavailable => "Unavailable",
    };
    let status = if transfer.status == Status::Transferring && !transfer.is_sender {
        "Receiving…"
    } else {
        status
    };
    let label = gtk::Label::new(Some(status));
    label.set_xalign(0.0);
    column.append(&label);
    if matches!(transfer.status, Status::Connecting | Status::Transferring) {
        let progress = gtk::ProgressBar::new();
        progress.set_fraction(if transfer.total_bytes == 0 {
            0.0
        } else {
            (transfer.transferred_bytes as f64 / transfer.total_bytes as f64).clamp(0.0, 1.0)
        });
        column.append(&progress);
    }
    if let Some(error) = transfer.error.as_deref().filter(|error| !error.is_empty()) {
        let label = gtk::Label::new(Some(error));
        label.set_wrap(true);
        label.set_max_width_chars(32);
        label.set_xalign(0.0);
        column.append(&label);
    }
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let action = |name: &str, value: AppAction| {
        let button = gtk::Button::with_label(name);
        button.set_widget_name(&format!("chatDirectTransfer{name}-{}", transfer.id));
        let manager = manager.clone();
        button.connect_clicked(move |_| manager.dispatch(value.clone()));
        actions.append(&button);
    };
    if transfer.status == Status::Offered && !transfer.is_sender {
        action(
            "Accept",
            AppAction::AcceptDirectFiles {
                chat_id: chat_id.into(),
                transfer_id: transfer.id.clone(),
            },
        );
        action(
            "Decline",
            AppAction::DeclineDirectFiles {
                chat_id: chat_id.into(),
                transfer_id: transfer.id.clone(),
            },
        );
    } else if matches!(
        transfer.status,
        Status::Offered | Status::Connecting | Status::Transferring
    ) {
        action(
            "Cancel",
            AppAction::CancelDirectFiles {
                chat_id: chat_id.into(),
                transfer_id: transfer.id.clone(),
            },
        );
    }
    column.append(&actions);
    column
}

fn size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

#[cfg(feature = "ui-tests")]
pub fn verify_ui(manager: Rc<AppManager>) {
    use iris_chat_core::DirectFileSnapshot;
    let mut transfer = DirectFileTransferSnapshot {
        id: "native-ui".into(),
        files: vec![
            DirectFileSnapshot {
                filename: "first.txt".into(),
                size_bytes: 10,
                local_path: None,
            },
            DirectFileSnapshot {
                filename: "second.txt".into(),
                size_bytes: 20,
                local_path: None,
            },
        ],
        status: DirectFileTransferStatus::Offered,
        is_sender: false,
        transferred_bytes: 0,
        total_bytes: 30,
        error: None,
    };
    fn has(widget: &gtk::Widget, name: &str) -> bool {
        if widget.widget_name() == name {
            return true;
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            if has(&current, name) {
                return true;
            }
            child = current.next_sibling();
        }
        false
    }
    let recipient = card("self-chat", &transfer, &manager);
    assert!(has(
        recipient.upcast_ref(),
        "chatDirectTransferAccept-native-ui"
    ));
    assert!(has(
        recipient.upcast_ref(),
        "chatDirectTransferDecline-native-ui"
    ));
    assert!(!has(
        recipient.upcast_ref(),
        "chatDirectTransferOpen-native-ui"
    ));
    transfer.is_sender = true;
    let sender = card("self-chat", &transfer, &manager);
    assert!(!has(
        sender.upcast_ref(),
        "chatDirectTransferAccept-native-ui"
    ));
    assert!(has(
        sender.upcast_ref(),
        "chatDirectTransferCancel-native-ui"
    ));
    transfer.status = DirectFileTransferStatus::Transferring;
    transfer.transferred_bytes = 15;
    let progress = card("self-chat", &transfer, &manager);
    let mut child = progress.first_child();
    let mut fraction = None;
    while let Some(widget) = child {
        if let Ok(bar) = widget.clone().downcast::<gtk::ProgressBar>() {
            fraction = Some(bar.fraction());
        }
        child = widget.next_sibling();
    }
    assert_eq!(fraction, Some(0.5));
    assert!(has(
        progress.upcast_ref(),
        "chatDirectTransferCancel-native-ui"
    ));
    assert!(!has(
        progress.upcast_ref(),
        "chatDirectTransferAccept-native-ui"
    ));
    transfer.status = DirectFileTransferStatus::Completed;
    transfer.files[0].local_path = Some("/tmp/direct-file-test.txt".into());
    let complete = card("self-chat", &transfer, &manager);
    assert!(has(
        complete.upcast_ref(),
        "chatDirectTransferOpen-native-ui"
    ));
    assert!(!has(
        complete.upcast_ref(),
        "chatDirectTransferCancel-native-ui"
    ));
    if let Some(path) = std::env::var_os("IRIS_DIRECT_FILES_UI_SCREENSHOT") {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 18);
        column.set_margin_top(20);
        column.set_margin_bottom(20);
        column.set_margin_start(20);
        column.set_margin_end(20);
        column.append(&recipient);
        column.append(&sender);
        column.append(&complete);
        let window = adw::Window::builder()
            .title("Direct files")
            .default_width(450)
            .default_height(620)
            .build();
        window.set_content(Some(&column));
        window.present();
        let context = gtk::glib::MainContext::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let paintable = gtk::WidgetPaintable::new(Some(&window));
        let mut node = None;
        while node.is_none() && std::time::Instant::now() < deadline {
            while context.pending() {
                context.iteration(false);
            }
            let snapshot = gtk::Snapshot::new();
            paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
            node = snapshot.to_node();
            if node.is_none() {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
        let texture = window
            .renderer()
            .expect("GTK renderer")
            .render_texture(&node.expect("direct files render"), None);
        texture.save_to_png(path).expect("direct files screenshot");
        window.close();
    }
    println!(
        "PASS: GTK direct file offer, recipient-device acceptance, multi-file completion controls"
    );
}
