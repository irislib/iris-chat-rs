use super::*;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::protocol::frame::{
    coding::{Data, OpCode},
    Frame,
};

#[tokio::test]
async fn production_transport_counts_raw_payloads_with_local_tls_roots() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = nostr::Url::parse(&format!("wss://{}", listener.local_addr().unwrap())).unwrap();
    let messages = vec![
        Message::Text("  [\"NOTICE\",\"ok\"]  ".into()),
        Message::Text("not JSON".into()),
        Message::Binary(vec![0, 255, 1]),
        Message::Text("not JSON".into()),
    ];
    let expected = messages.iter().map(Message::len).sum::<usize>();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let tls = super::super::super::anonymous_reads::tls_tests::acceptor(false, false)
            .accept(tcp)
            .await
            .unwrap();
        let mut socket = tokio_tungstenite::accept_async(tls).await.unwrap();
        for message in messages {
            socket.send(message).await.unwrap();
        }
        let _ = socket.close(None).await;
    });
    let transport = DiscoveryTransport::trusted_test_roots(
        super::super::super::anonymous_reads::tls_tests::roots(),
    );
    let counter = transport
        .counters
        .counter(&RelayUrl::parse(url.as_str()).unwrap())
        .unwrap();
    let (_, mut receive) = transport
        .connect(
            &url,
            &nostr_sdk::pool::ConnectionMode::Direct,
            Duration::from_secs(3),
        )
        .await
        .unwrap();
    for _ in 0..4 {
        receive.next().await.unwrap().unwrap();
    }
    assert_eq!(counter.load(Ordering::Relaxed), expected);
    server.await.unwrap();
}

#[tokio::test]
async fn production_transport_rejects_raw_padded_frame_and_fragmented_message_over_limit() {
    for fragmented in [false, true] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = nostr::Url::parse(&format!("ws://{}", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
            if fragmented {
                let data = vec![b' '; MAX_MESSAGE_BYTES / 2 + 1];
                let _ = socket
                    .send(Message::Frame(Frame::message(
                        data.clone(),
                        OpCode::Data(Data::Text),
                        false,
                    )))
                    .await;
                let _ = socket
                    .send(Message::Frame(Frame::message(
                        data,
                        OpCode::Data(Data::Continue),
                        true,
                    )))
                    .await;
            } else {
                let _ = socket
                    .send(Message::Text(format!(
                        "{}[\"NOTICE\",\"ok\"]",
                        " ".repeat(MAX_MESSAGE_BYTES)
                    )))
                    .await;
            }
        });
        let transport = DiscoveryTransport::default();
        let (_, mut receive) = transport
            .connect(
                &url,
                &nostr_sdk::pool::ConnectionMode::Direct,
                Duration::from_secs(3),
            )
            .await
            .unwrap();
        assert!(tokio::time::timeout(Duration::from_secs(3), receive.next())
            .await
            .unwrap()
            .unwrap()
            .is_err());
        // After rejecting the frame header, the receiver deliberately stops
        // draining its payload. Close that socket before joining the writer.
        drop(receive);
        tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .unwrap()
            .unwrap();
    }
}

#[tokio::test]
async fn unsupported_proxy_mode_is_rejected_before_dialing() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let url = nostr::Url::parse(&format!("ws://{address}")).unwrap();
    let result = DiscoveryTransport::default()
        .connect(
            &url,
            &nostr_sdk::pool::ConnectionMode::Proxy(address),
            Duration::from_secs(1),
        )
        .await;
    assert!(result.is_err());
    assert!(
        tokio::time::timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err()
    );
}
