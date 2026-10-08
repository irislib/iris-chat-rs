use adw::prelude::*;
use iris_chat_core::{AppState, DeviceHistorySyncPhase};

pub(super) struct SyncStatus {
    pub root: gtk::Box,
    spinner: gtk::Spinner,
    waiting: gtk::Image,
    label: gtk::Label,
}

impl SyncStatus {
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        root.set_widget_name("message-sync-status");
        root.set_margin_start(20);
        root.set_margin_end(20);
        root.set_margin_top(10);
        root.set_margin_bottom(4);
        let spinner = gtk::Spinner::new();
        let waiting = gtk::Image::from_icon_name("media-playback-pause-symbolic");
        let label = gtk::Label::new(None);
        label.add_css_class("dim-label");
        label.add_css_class("caption");
        label.set_xalign(0.0);
        label.set_wrap(true);
        root.append(&spinner);
        root.append(&waiting);
        root.append(&label);
        Self {
            root,
            spinner,
            waiting,
            label,
        }
    }

    pub fn update(&self, state: &AppState) {
        let history = state
            .device_history_sync
            .as_ref()
            .filter(|sync| sync.phase != DeviceHistorySyncPhase::Complete);
        let visible = state.account.is_some() && (history.is_some() || state.busy.syncing_network);
        let waiting = history.is_some_and(|sync| sync.phase == DeviceHistorySyncPhase::Waiting);
        let text = if waiting {
            "Waiting for your other device…".to_owned()
        } else if let Some((done, total)) = history.and_then(|sync| {
            sync.total_messages
                .filter(|n| *n > 0)
                .map(|n| (sync.imported_messages, n))
        }) {
            format!("Syncing messages… {done} of {total}")
        } else {
            "Syncing messages…".to_owned()
        };
        self.label.set_label(&text);
        self.root.set_visible(visible);
        self.waiting.set_visible(waiting);
        self.spinner.set_visible(!waiting);
        self.spinner.set_spinning(visible && !waiting);
    }
}
