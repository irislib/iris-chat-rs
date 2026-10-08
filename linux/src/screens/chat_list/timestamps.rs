use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;

#[derive(Clone, Default)]
pub(super) struct Timestamps {
    labels: Rc<RefCell<Vec<(u64, glib::WeakRef<gtk::Label>)>>>,
}

impl Timestamps {
    pub(super) fn install(root: &gtk::Box) -> Self {
        let timestamps = Self::default();
        let clock = timestamps.clone();
        let weak = root.downgrade();
        let source = glib::timeout_add_local(Duration::from_secs(30), move || {
            if weak.upgrade().is_none() {
                return glib::ControlFlow::Break;
            }
            clock.refresh(super::unix_now());
            glib::ControlFlow::Continue
        });
        // One clock per sidebar; rows remain mounted, and neither the timer
        // nor the timestamp registry keeps discarded widgets alive.
        let source = RefCell::new(Some(source));
        root.connect_destroy(move |_| {
            if let Some(source) = source.borrow_mut().take() {
                source.remove();
            }
        });
        timestamps
    }

    pub(super) fn clear(&self) {
        self.labels.borrow_mut().clear();
    }

    pub(super) fn label(&self, timestamp: u64, now: u64) -> gtk::Label {
        let label = gtk::Label::new(Some(&super::relative_time(timestamp, now)));
        self.labels
            .borrow_mut()
            .push((timestamp, label.downgrade()));
        label
    }

    pub(super) fn refresh(&self, now: u64) {
        self.labels.borrow_mut().retain(|(timestamp, weak)| {
            let Some(label) = weak.upgrade() else {
                return false;
            };
            let text = super::relative_time(*timestamp, now);
            if label.text().as_str() != text {
                label.set_label(&text);
            }
            true
        });
    }
}

#[cfg(feature = "ui-tests")]
mod tests;
#[cfg(feature = "ui-tests")]
pub use tests::verify_ui;
