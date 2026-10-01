// Native GTK widgets and the exported production FIPS-TCP transport fixture.
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
    let result =
        iris_chat_core::run_direct_file_transfer_smoke(data.path().to_string_lossy().into_owned());
    println!("{result}");
    let report: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(report["ok"], true, "{result}");
    assert_eq!(report["files"].as_array().unwrap().len(), 3);
    assert_eq!(report["bytes_before_accept"], 0);
    assert_eq!(std::fs::read_dir(data.path()).unwrap().count(), 0);
    std::env::set_var("IRIS_UI_TEST_RUN_ID", "native-direct-files");
    std::env::set_var("IRIS_UI_TEST_DATA_DIR", data.path());
    std::env::set_var("IRIS_DEMO_RELAYS", "ws://127.0.0.1:9");
    std::env::set_var("IRIS_FIPS_WEBSOCKET_SEED_URLS", "");
    std::env::set_var("XDG_CONFIG_HOME", data.path().join("config"));
    std::env::set_var("XDG_DATA_HOME", data.path().join("data"));
    adw::init().expect("GTK requires a native display or Xvfb");
    screens::chat::verify_direct_files_ui(std::rc::Rc::new(app_manager::AppManager::new()));
}
