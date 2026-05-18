use gtk::glib::object::IsA;
use gtk::prelude::*;

pub trait PointerCursorExt: IsA<gtk::Widget> {
    fn show_pointer_cursor(&self) {
        self.set_cursor_from_name(Some("pointer"));
    }
}

impl<T: IsA<gtk::Widget>> PointerCursorExt for T {}
