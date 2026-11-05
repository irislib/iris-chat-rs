//! Bounded public sockets. Authentication and subscription ownership stay in
//! the SDK; raw payload accounting happens before its JSON normalization.
use super::*;
use futures_util::{stream::SplitSink, Sink, StreamExt};
use nostr_sdk::pool::transport::{
    error::TransportError,
    websocket::{WebSocketSink, WebSocketStream as SdkStream, WebSocketTransport},
};
use std::pin::Pin;
use std::sync::atomic::AtomicUsize;
use std::task::{Context, Poll};
use tokio_tungstenite::{
    tungstenite::{
        protocol::{CloseFrame, WebSocketConfig},
        Message,
    },
    Connector, MaybeTlsStream, WebSocketStream,
};

mod dial;
#[cfg(test)]
mod tests;

pub(super) const MAX_GRAPH_EVENT_BYTES: usize = 1024 * 1024;
pub(super) const MAX_MESSAGE_BYTES: usize = MAX_GRAPH_EVENT_BYTES + 4096;

#[derive(Debug, Default)]
pub(super) struct WireCounters {
    // Active scopes/connections retain their Arc through reconnects. Only idle
    // slots can be replaced, keeping repeated server edits bounded.
    relays: std::sync::Mutex<BTreeMap<RelayUrl, Arc<AtomicUsize>>>,
}
impl WireCounters {
    pub(super) fn counter(&self, url: &RelayUrl) -> Result<Arc<AtomicUsize>, String> {
        let mut relays = self
            .relays
            .lock()
            .map_err(|_| "public counters unavailable")?;
        if let Some(counter) = relays.get(url) {
            return Ok(counter.clone());
        }
        if relays.len() == 64 {
            let idle = relays
                .iter()
                .find(|(_, counter)| Arc::strong_count(counter) == 1)
                .map(|(url, _)| url.clone())
                .ok_or("too many active public servers")?;
            relays.remove(&idle);
        }
        let counter = Arc::new(AtomicUsize::new(0));
        relays.insert(url.clone(), counter.clone());
        Ok(counter)
    }
}

pub(super) struct DiscoveryTransport {
    pub(super) counters: Arc<WireCounters>,
    tls: Arc<rustls::ClientConfig>,
}
impl std::fmt::Debug for DiscoveryTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DiscoveryTransport")
    }
}
impl Default for DiscoveryTransport {
    fn default() -> Self {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        Self::with_roots(roots)
    }
}
impl DiscoveryTransport {
    fn with_roots(roots: rustls::RootCertStore) -> Self {
        Self {
            counters: Default::default(),
            tls: Arc::new(
                rustls::ClientConfig::builder()
                    .with_root_certificates(roots)
                    .with_no_client_auth(),
            ),
        }
    }
    #[cfg(test)]
    pub(super) fn trusted_test_roots(roots: rustls::RootCertStore) -> Self {
        Self::with_roots(roots)
    }
}

impl WebSocketTransport for DiscoveryTransport {
    fn support_ping(&self) -> bool {
        true
    }
    fn connect<'a>(
        &'a self,
        url: &'a nostr::Url,
        mode: &'a nostr_sdk::pool::ConnectionMode,
        timeout: Duration,
    ) -> nostr::util::BoxedFuture<'a, Result<(WebSocketSink, SdkStream), TransportError>> {
        Box::pin(async move {
            // A future proxy configuration must never silently become a direct
            // connection. This public pool currently configures Direct only.
            if mode != &nostr_sdk::pool::ConnectionMode::Direct {
                return Err(TransportError::backend(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "public discovery connection mode is unsupported",
                )));
            }
            let relay = RelayUrl::parse(url.as_str()).map_err(TransportError::backend)?;
            let counter = self
                .counters
                .counter(&relay)
                .map_err(|error| TransportError::backend(std::io::Error::other(error)))?;
            let connector = Some(Connector::Rustls(self.tls.clone()));
            let config = WebSocketConfig {
                write_buffer_size: 0,
                max_write_buffer_size: MAX_MESSAGE_BYTES * 2,
                max_message_size: Some(MAX_MESSAGE_BYTES),
                max_frame_size: Some(MAX_MESSAGE_BYTES),
                ..Default::default()
            };
            let operation = async {
                let tcp = dial::connect(url).await.map_err(TransportError::backend)?;
                // Original URL supplies HTTP Host, TLS SNI and WebPKI checks.
                // Box this large handshake future as the existing SDK does on Windows.
                let (socket, _) = Box::pin(tokio_tungstenite::client_async_tls_with_config(
                    url.as_str(),
                    tcp,
                    Some(config),
                    connector,
                ))
                .await
                .map_err(TransportError::backend)?;
                Ok::<_, TransportError>(socket)
            };
            let socket = tokio::time::timeout(timeout, operation)
                .await
                .map_err(TransportError::backend)??;
            let (send, receive) = socket.split();
            let sink: WebSocketSink = Box::new(TransportSink(send));
            let stream: SdkStream = Box::pin(receive.map(move |result| {
                result.map_err(TransportError::backend).and_then(|message| {
                    // Count complete raw WebSocket payloads, including padding,
                    // duplicate/invalid JSON and binary messages, before parsing.
                    // TLS/frame headers and rejected oversized partial messages
                    // are not represented here; this is not a network-byte meter.
                    counter.fetch_add(message.len(), Ordering::Relaxed);
                    match message {
                        Message::Text(text) => Ok(async_wsocket::Message::Text(text)),
                        Message::Binary(data) => Ok(async_wsocket::Message::Binary(data)),
                        Message::Ping(data) => Ok(async_wsocket::Message::Ping(data)),
                        Message::Pong(data) => Ok(async_wsocket::Message::Pong(data)),
                        Message::Close(frame) => {
                            Ok(async_wsocket::Message::Close(frame.map(|frame| {
                                async_wsocket::message::CloseFrame {
                                    code: frame.code.into(),
                                    reason: frame.reason.into_owned(),
                                }
                            })))
                        }
                        Message::Frame(_) => Err(TransportError::backend(std::io::Error::other(
                            "unexpected raw WebSocket frame",
                        ))),
                    }
                })
            }));
            Ok((sink, stream))
        })
    }
}

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
struct TransportSink(SplitSink<Socket, Message>);
// Explicit forwarding avoids the SDK's documented sink_map_err panic (#984).
impl Sink<async_wsocket::Message> for TransportSink {
    type Error = TransportError;
    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.0)
            .poll_ready(cx)
            .map_err(TransportError::backend)
    }
    fn start_send(
        mut self: Pin<&mut Self>,
        item: async_wsocket::Message,
    ) -> Result<(), Self::Error> {
        let item = match item {
            async_wsocket::Message::Text(text) => Message::Text(text),
            async_wsocket::Message::Binary(data) => Message::Binary(data),
            async_wsocket::Message::Ping(data) => Message::Ping(data),
            async_wsocket::Message::Pong(data) => Message::Pong(data),
            async_wsocket::Message::Close(frame) => Message::Close(frame.map(|frame| CloseFrame {
                code: frame.code.into(),
                reason: frame.reason.into(),
            })),
        };
        Pin::new(&mut self.0)
            .start_send(item)
            .map_err(TransportError::backend)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.0)
            .poll_flush(cx)
            .map_err(TransportError::backend)
    }
    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.0)
            .poll_close(cx)
            .map_err(TransportError::backend)
    }
}
