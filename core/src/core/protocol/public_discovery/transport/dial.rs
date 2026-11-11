//! Public TCP dialing with a bounded Happy Eyeballs race. The two address
//! families retain resolver order; TLS/authentication state lives elsewhere.
use std::{io, net::SocketAddr, time::Duration};
use tokio::net::TcpStream;

async fn family(addresses: Vec<SocketAddr>) -> io::Result<TcpStream> {
    let mut error = io::Error::new(
        io::ErrorKind::AddrNotAvailable,
        "no public server addresses",
    );
    for address in addresses {
        match TcpStream::connect(address).await {
            Ok(socket) => return Ok(socket),
            Err(next) => error = next,
        }
    }
    Err(error)
}

pub(super) async fn connect(url: &nostr::Url) -> io::Result<TcpStream> {
    if !matches!(url.scheme(), "ws" | "wss") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unsupported public server scheme",
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| io::Error::other("public server has no host"))?
        .trim_start_matches('[')
        .trim_end_matches(']');
    let port = url
        .port_or_known_default()
        .ok_or_else(|| io::Error::other("public server has no port"))?;
    let addresses = tokio::net::lookup_host((host, port))
        .await?
        .collect::<Vec<_>>();
    let prefer_v6 = addresses.first().is_some_and(SocketAddr::is_ipv6);
    let (first, second): (Vec<_>, Vec<_>) = addresses
        .into_iter()
        .partition(|address| address.is_ipv6() == prefer_v6);
    if second.is_empty() {
        return family(first).await;
    }
    let first = family(first);
    tokio::pin!(first);
    // One family's connection attempts never hold up the other for longer than
    // 250ms; failed attempts advance within that family. Cancelling the outer
    // connector drops both futures and their sockets without detached work.
    tokio::select! {
        result = &mut first => if result.is_ok() { result } else { family(second).await },
        _ = tokio::time::sleep(Duration::from_millis(250)) => {
            let second = family(second);
            tokio::pin!(second);
            tokio::select! {
                result = &mut first => if result.is_ok() { result } else { second.await },
                result = &mut second => if result.is_ok() { result } else { first.await },
            }
        }
    }
}
