//! Private observation-only diagnostic. Not a release dependency or policy knob.
#![allow(dead_code)]
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

const LIMIT: usize = 256;
static ROLE: OnceLock<Option<String>> = OnceLock::new();
static SEQUENCE: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn enabled() -> bool {
    role().is_some()
}

fn role() -> Option<&'static str> {
    ROLE.get_or_init(|| {
        std::env::var("FLEET_UPDATER_TRACE")
            .ok()
            .filter(|role| matches!(role.as_str(), "client" | "provider"))
    })
    .as_deref()
}

fn record(
    role: &str,
    phase: &str,
    peer: Option<&str>,
    value: u64,
    sequence: usize,
    unix_ms: u128,
) -> Option<String> {
    if !matches!(role, "client" | "provider")
        || phase.is_empty()
        || phase.len() > 64
        || !phase.bytes().all(|c| c.is_ascii_lowercase() || c == b'_')
        || sequence > LIMIT
    {
        return None;
    }
    let phase = if sequence == LIMIT {
        "trace_limit_reached"
    } else {
        phase
    };
    let mut hasher = DefaultHasher::new();
    peer.unwrap_or("").hash(&mut hasher);
    // Correlation only: never use this non-cryptographic label as authentication.
    Some(format!(
        "{{\"fleet_updater_trace\":1,\"role\":\"{role}\",\"phase\":\"{phase}\",\"peer_label\":\"{:016x}\",\"value\":{value},\"sequence\":{sequence},\"unix_ms\":{unix_ms}}}",
        hasher.finish()
    ))
}

pub(crate) fn emit(phase: &'static str, peer: Option<&str>, value: u64) {
    let Some(role) = role() else { return };
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    if sequence > LIMIT {
        return;
    }
    let unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    if let Some(record) = record(role, phase, peer, value, sequence, unix_ms) {
        // A closed diagnostic sink must not change transport behavior.
        let _ = writeln!(std::io::stderr().lock(), "{record}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omits_peer_identity_and_dynamic_text() {
        let line = record(
            "client",
            "req_received",
            Some("private-peer-identity"),
            3,
            0,
            17,
        )
        .unwrap();
        assert!(!line.contains("private-peer-identity"));
        assert!(line.contains("\"value\":3"));
        assert!(record("client", "injected\"payload", None, 0, 0, 0).is_none());
        assert!(record("arbitrary-role", "ok", None, 0, 0, 0).is_none());
    }

    #[test]
    fn label_correlates_without_claiming_authentication() {
        let left = record("client", "tcp_connected", Some("same-peer"), 1, 0, 0).unwrap();
        let right = record("client", "tcp_connected", Some("same-peer"), 1, 0, 0).unwrap();
        assert_eq!(left, right);
        assert_ne!(
            left,
            record("client", "tcp_connected", Some("other-peer"), 1, 0, 0).unwrap()
        );
    }

    #[test]
    fn emits_explicit_limit_then_silences() {
        assert!(record("provider", "reply_queued", None, 1, LIMIT, 0)
            .unwrap()
            .contains("trace_limit_reached"));
        assert!(record("provider", "reply_queued", None, 1, LIMIT + 1, 0).is_none());
    }
}
