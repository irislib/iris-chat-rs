use super::*;
use std::time::Duration;

async fn endpoint() -> Arc<FipsEndpoint> {
    let mut config = fips_core::Config::new();
    config.node.control.enabled = false;
    config.node.discovery.lan.enabled = false;
    config.node.discovery.nostr.enabled = false;
    Arc::new(
        FipsEndpoint::builder()
            .config(config)
            .without_system_tun()
            .bind()
            .await
            .unwrap(),
    )
}

#[tokio::test]
async fn service_epoch_requires_same_endpoint_generation_and_survives_snapshots() {
    let first = endpoint().await;
    let second = endpoint().await;
    let mut history = None;
    let now = Instant::now();
    let record = |endpoint, history: &mut _, generation, instant| {
        Observation::capture(endpoint, None)
            .unwrap()
            .record(history, generation, instant)
    };
    let before = record(first.clone(), &mut history, 1, now);
    let after = record(first.clone(), &mut history, 1, now + Duration::from_secs(2));
    assert_eq!(before["epoch_id"], after["epoch_id"]);
    assert_ne!(before["sample_id"], after["sample_id"]);
    assert_eq!(after["elapsed_ms"], 2000);
    assert_eq!(after["services"].as_object().unwrap().len(), 2);
    assert!(after["pubsub_delivery"].is_null());
    assert!(!after.to_string().contains(first.npub()));
    let replaced = record(
        second.clone(),
        &mut history,
        1,
        now + Duration::from_secs(3),
    );
    assert_ne!(after["epoch_id"], replaced["epoch_id"]);
    let generation = record(
        second.clone(),
        &mut history,
        2,
        now + Duration::from_secs(4),
    );
    assert_ne!(replaced["epoch_id"], generation["epoch_id"]);
    first.shutdown().await.unwrap();
    second.shutdown().await.unwrap();
}

#[tokio::test]
async fn service_counter_overflow_and_clock_regression_discard_epoch() {
    let endpoint = endpoint().await;
    let mut history = None;
    let now = Instant::now();
    Observation::capture(endpoint.clone(), None)
        .unwrap()
        .record(&mut history, 1, now);
    let backwards = Observation::capture(endpoint.clone(), None)
        .unwrap()
        .record(&mut history, 1, now - Duration::from_millis(1));
    assert_eq!(backwards["status"], "invalid_counters");
    assert!(history.is_none());
    let mut overflow = Observation::capture(endpoint.clone(), None).unwrap();
    overflow
        .services
        .get_mut("hashtree")
        .unwrap()
        .discarded_outputs = u64::MAX;
    let overflow = overflow.record(&mut history, 1, now);
    assert_eq!(overflow["valid"], false);
    assert!(overflow.get("services").is_none());
    assert!(history.is_none());
    endpoint.shutdown().await.unwrap();
}

#[tokio::test]
async fn pubsub_availability_changes_epoch_and_never_fabricates_zero_delivery() {
    let endpoint = endpoint().await;
    let mut history = None;
    let before = Observation::capture(endpoint.clone(), None)
        .unwrap()
        .record(&mut history, 1, Instant::now());
    let pubsub = Arc::new(
        FipsPubsubClient::start(endpoint.clone(), Default::default())
            .await
            .unwrap(),
    );
    let available = Observation::capture(endpoint.clone(), Some(pubsub.clone()))
        .unwrap()
        .record(&mut history, 1, Instant::now());
    assert_ne!(before["epoch_id"], available["epoch_id"]);
    assert_eq!(available["pubsub_delivery"].as_object().unwrap().len(), 14);
    let repeated = Observation::capture(endpoint.clone(), Some(pubsub.clone()))
        .unwrap()
        .record(&mut history, 1, Instant::now());
    assert_eq!(available["epoch_id"], repeated["epoch_id"]);
    let absent = Observation::capture(endpoint.clone(), None)
        .unwrap()
        .record(&mut history, 1, Instant::now());
    assert_ne!(available["epoch_id"], absent["epoch_id"]);
    assert!(absent["pubsub_delivery"].is_null());
    pubsub.shutdown_shared().await;
    endpoint.shutdown().await.unwrap();
}
