use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// A real local Blossom endpoint for CLI invite upload/download. Shared only
/// within this test process, with no changes to the user's hashtree config.
pub fn invite_server_config() -> PathBuf {
    static FIXTURE: OnceLock<tempfile::TempDir> = OnceLock::new();
    FIXTURE
        .get_or_init(|| {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let blobs = Arc::new(Mutex::new(HashMap::<String, Vec<u8>>::new()));
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let blobs = blobs.clone();
                    std::thread::spawn(move || {
                        let _ = serve(stream, &blobs);
                    });
                }
            });
            let directory = tempfile::tempdir().unwrap();
            std::fs::write(
                directory.path().join("config.toml"),
                format!("[blossom]\nread_servers = [\"{url}\"]\nwrite_servers = [\"{url}\"]\n"),
            )
            .unwrap();
            directory
        })
        .path()
        .to_owned()
}

fn serve(mut stream: TcpStream, blobs: &Mutex<HashMap<String, Vec<u8>>>) -> std::io::Result<()> {
    use base64::Engine;
    use nostr::JsonUtil;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request = String::new();
    reader.read_line(&mut request)?;
    let mut headers = HashMap::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        if line == "\r\n" || line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.trim_end().split_once(':') {
            headers.insert(key.to_ascii_lowercase(), value.trim().to_owned());
        }
        if headers.len() > 40 {
            return Ok(());
        }
    }
    let parts: Vec<_> = request.split_whitespace().collect();
    let (status, body) = if parts.first() == Some(&"PUT") && parts.get(1) == Some(&"/upload") {
        let size = headers
            .get("content-length")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0);
        if !(16..=16 * 1024).contains(&size) {
            return Ok(());
        }
        let mut data = vec![0; size];
        reader.read_exact(&mut data)?;
        let hash = hashtree_core::to_hex(&hashtree_core::sha256(&data));
        let verified = headers
            .get("authorization")
            .and_then(|s| s.strip_prefix("Nostr "))
            .and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok())
            .and_then(|s| nostr::Event::from_json(s).ok())
            .is_some_and(|event| {
                event.verify().is_ok()
                    && event.kind.as_u16() == 24242
                    && event
                        .tags
                        .iter()
                        .any(|tag| tag.as_slice() == ["t", "upload"])
                    && event
                        .tags
                        .iter()
                        .any(|tag| tag.as_slice() == ["x", hash.as_str()])
            });
        if verified {
            blobs.lock().unwrap().insert(hash.clone(), data);
            (
                "201 Created",
                serde_json::json!({"sha256":hash, "size":size})
                    .to_string()
                    .into_bytes(),
            )
        } else {
            ("403 Forbidden", Vec::new())
        }
    } else {
        let data = parts.get(1).and_then(|path| {
            blobs
                .lock()
                .unwrap()
                .get(path.trim_start_matches('/'))
                .cloned()
        });
        match data {
            Some(data) => ("200 OK", data),
            None => ("404 Not Found", Vec::new()),
        }
    };
    write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n", body.len())?;
    stream.write_all(&body)
}
