use std::env;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const FALLBACK_RELAYS: &str = "wss://relay.damus.io,wss://nos.lol,wss://relay.primal.net,wss://relay.snort.social,wss://temp.iris.to";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    for key in [
        "NDR_APP_VERSION",
        "NDR_BUILD_CHANNEL",
        "NDR_BUILD_GIT_SHA",
        "NDR_BUILD_TIMESTAMP_UTC",
        "NDR_DEFAULT_RELAYS",
        "NDR_RELAY_SET_ID",
        "NDR_TRUSTED_TEST_BUILD",
        "SOURCE_DATE_EPOCH",
    ] {
        println!("cargo:rerun-if-env-changed={key}");
    }

    emit(
        "NDR_APP_VERSION",
        env::var("NDR_APP_VERSION").unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string()),
    );
    emit(
        "NDR_BUILD_CHANNEL",
        env::var("NDR_BUILD_CHANNEL").unwrap_or_else(|_| "debug".to_string()),
    );
    emit(
        "NDR_BUILD_GIT_SHA",
        env::var("NDR_BUILD_GIT_SHA").unwrap_or_else(|_| detect_git_sha()),
    );
    emit(
        "NDR_BUILD_TIMESTAMP_UTC",
        env::var("NDR_BUILD_TIMESTAMP_UTC")
            .or_else(|_| env::var("SOURCE_DATE_EPOCH"))
            .unwrap_or_else(|_| detect_git_timestamp()),
    );
    emit(
        "NDR_DEFAULT_RELAYS",
        env::var("NDR_DEFAULT_RELAYS").unwrap_or_else(|_| FALLBACK_RELAYS.to_string()),
    );
    emit(
        "NDR_RELAY_SET_ID",
        env::var("NDR_RELAY_SET_ID").unwrap_or_else(|_| "public-dev".to_string()),
    );
    emit(
        "NDR_TRUSTED_TEST_BUILD",
        env::var("NDR_TRUSTED_TEST_BUILD").unwrap_or_else(|_| "false".to_string()),
    );
}

fn emit(key: &str, value: String) {
    println!("cargo:rustc-env={key}={value}");
}

fn detect_git_sha() -> String {
    Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
            } else {
                None
            }
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn detect_git_timestamp() -> String {
    Command::new("git")
        .args(["log", "-1", "--format=%ct", "HEAD"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
            } else {
                None
            }
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(unix_timestamp_string)
}

fn unix_timestamp_string() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}
