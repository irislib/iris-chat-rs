use adw::prelude::*;
use gtk::{gdk, glib};

pub(super) fn install(picture: &gtk::Picture) -> gtk::Popover {
    let popover = gtk::Popover::new();
    popover.set_has_arrow(false);
    popover.set_parent(picture);
    let copy = gtk::Button::with_label("Copy image");
    copy.add_css_class("flat");
    copy.set_margin_top(6);
    copy.set_margin_bottom(6);
    copy.set_margin_start(6);
    copy.set_margin_end(6);
    popover.set_child(Some(&copy));
    let weak_picture = picture.downgrade();
    let weak_popover = popover.downgrade();
    copy.connect_clicked(move |_| {
        if let Some(picture) = weak_picture.upgrade() {
            if let Some(texture) = picture
                .paintable()
                .and_then(|p| p.downcast::<gdk::Texture>().ok())
            {
                picture.clipboard().set_texture(&texture);
            }
        }
        if let Some(popover) = weak_popover.upgrade() {
            popover.popdown();
        }
    });
    let gesture = gtk::GestureClick::new();
    gesture.set_button(3);
    let weak_picture = picture.downgrade();
    let weak_popover = popover.downgrade();
    gesture.connect_pressed(move |gesture, _, x, y| {
        let (Some(picture), Some(popover)) = (weak_picture.upgrade(), weak_popover.upgrade())
        else {
            return;
        };
        copy.set_sensitive(picture.paintable().is_some_and(|p| p.is::<gdk::Texture>()));
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.popup();
        gesture.set_state(gtk::EventSequenceState::Claimed);
    });
    picture.add_controller(gesture);
    let weak_popover = popover.downgrade();
    picture.connect_destroy(move |_| {
        if let Some(popover) = weak_popover.upgrade() {
            popover.unparent();
        }
    });
    popover
}

#[cfg(feature = "ui-tests")]
pub fn verify_ui() {
    let bytes = glib::Bytes::from_owned(vec![255, 0, 0, 255, 0, 255, 0, 255]);
    let texture = gdk::MemoryTexture::new(2, 1, gdk::MemoryFormat::R8g8b8a8, &bytes, 8);
    let picture = gtk::Picture::for_paintable(&texture);
    let popover = install(&picture);
    popover
        .child()
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap()
        .emit_clicked();
    let copied = glib::MainContext::default()
        .block_on(picture.clipboard().read_texture_future())
        .unwrap()
        .unwrap();
    assert_eq!((copied.width(), copied.height()), (2, 1));
    let mut pixels = vec![0; 8];
    copied.download(&mut pixels, 8);
    // GDK downloads native ARGB32 (BGRA on our little-endian platforms).
    assert_eq!(pixels, vec![0, 0, 255, 255, 0, 255, 0, 255]);
    popover.unparent();
    println!("PASS: image viewer Copy image puts pixels on the clipboard");
}
