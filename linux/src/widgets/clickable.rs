use gtk::glib::object::IsA;
use gtk::prelude::*;

pub trait PointerCursorExt: IsA<gtk::Widget> {
    fn show_pointer_cursor(&self) {
        self.set_cursor_from_name(Some(if self.is_sensitive() {
            "pointer"
        } else {
            "default"
        }));
        self.connect_state_flags_changed(|widget, _| {
            widget.set_cursor_from_name(Some(if widget.is_sensitive() {
                "pointer"
            } else {
                "default"
            }));
        });
    }
}

impl<T: IsA<gtk::Widget>> PointerCursorExt for T {}
