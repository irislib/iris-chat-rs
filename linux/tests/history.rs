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
    let data = tempfile::tempdir().unwrap();
    std::env::set_var("IRIS_UI_TEST_RUN_ID", "linux-history-regression");
    std::env::set_var("IRIS_UI_TEST_DATA_DIR", data.path());
    std::env::set_var("IRIS_DEMO_RELAYS", "ws://127.0.0.1:9");
    std::env::set_var("IRIS_FIPS_WEBSOCKET_SEED_URLS", "");
    std::env::set_var("XDG_CONFIG_HOME", data.path().join("config"));
    std::env::set_var("XDG_DATA_HOME", data.path().join("data"));
    adw::init().expect("GTK display required");
    app_manager::verify_history_ui(std::rc::Rc::new(app_manager::AppManager::new()));
}
