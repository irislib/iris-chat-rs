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

fn main() {
    window::composer_tests::run();
}
