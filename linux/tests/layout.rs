// A separate main-thread runner is required by GTK's macOS backend too.
#[path = "../src/app_manager.rs"]
mod app_manager;
#[path = "../src/calls.rs"]
mod calls;
#[path = "../src/platform/mod.rs"]
mod platform;
#[path = "../src/screens/mod.rs"]
mod screens;
#[path = "../src/secure_storage.rs"]
mod secure_storage;
#[path = "../src/widgets/mod.rs"]
mod widgets;
#[path = "../src/window.rs"]
mod window;

#[path = "../src/style.rs"]
mod style;

fn main() {
    adw::init().expect("GTK display required");
    style::install_css();
    window::composer_tests::run_layout();
}
