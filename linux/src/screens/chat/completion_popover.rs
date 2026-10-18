use gtk::prelude::*;

pub(super) fn attach(
    input: &gtk::TextView,
    root: &gtk::Box,
    anchor: &impl IsA<gtk::Widget>,
) -> gtk::ScrolledWindow {
    // Keep names readable even when the ellipsized rows request a narrow size.
    root.set_size_request(220, -1);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_min_content_width(220);
    scroll.set_max_content_height(240);
    scroll.set_propagate_natural_height(true);
    scroll.set_child(Some(root));
    let popup = gtk::Popover::new();
    popup.set_position(gtk::PositionType::Top);
    popup.set_autohide(false);
    popup.set_has_arrow(false);
    popup.set_focusable(false);
    // Keyboard navigation belongs to the editor. Excluding the whole
    // subtree also prevents popup() from focusing the first result row.
    popup.set_can_focus(false);
    popup.set_child(Some(&scroll));
    // Use a layout-managed anchor: TextView has a private child allocator.
    popup.set_parent(anchor);
    // Read the child's own visibility property. is_visible() also checks
    // the hidden popover ancestor and would prevent the popup ever opening.
    let showing = popup.downgrade();
    root.connect_visible_notify(move |root| {
        if let Some(popup) = showing.upgrade() {
            if root.get_visible() {
                popup.popup();
            } else {
                popup.popdown();
            }
        }
    });
    let cleanup = popup.downgrade();
    input.connect_destroy(move |_| {
        if let Some(popup) = cleanup.upgrade() {
            popup.unparent();
        }
    });
    scroll
}
