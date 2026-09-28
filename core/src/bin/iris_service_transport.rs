//! Private, same-user IPC. No TCP listener and no encryption-state copy.
use anyhow::{Context, Result};
use interprocess::local_socket::{
    prelude::*, Listener, ListenerNonblockingMode, ListenerOptions, Name, Stream,
};
use serde_json::Value;
use std::{
    io::{self, Read, Write},
    path::Path,
    time::{Duration, Instant},
};

pub(super) fn name(data_dir: &Path) -> Result<Name<'static>> {
    #[cfg(unix)]
    {
        use interprocess::local_socket::GenericFilePath;
        Ok(data_dir.join("cli.sock").to_fs_name::<GenericFilePath>()?)
    }
    #[cfg(windows)]
    {
        use interprocess::local_socket::GenericNamespaced;
        use sha2::{Digest, Sha256};
        let path = data_dir.canonicalize()?;
        let digest = Sha256::digest(path.as_os_str().as_encoded_bytes());
        Ok(format!("iris-cli-{digest:x}").to_ns_name::<GenericNamespaced>()?)
    }
}

// Called only after CliApp acquired the existing exclusive profile guard.
pub(super) fn bind(data_dir: &Path) -> Result<Listener> {
    let options = ListenerOptions::new()
        .name(name(data_dir)?)
        .nonblocking(ListenerNonblockingMode::Both);
    #[cfg(unix)]
    let options = {
        use std::os::unix::fs::{FileTypeExt, MetadataExt};
        let socket = data_dir.join("cli.sock");
        match std::fs::symlink_metadata(&socket) {
            Ok(m) => {
                anyhow::ensure!(
                    m.file_type().is_socket() && m.uid() == std::fs::metadata(data_dir)?.uid(),
                    "Refusing to replace a non-socket or foreign service endpoint"
                );
                std::fs::remove_file(socket)?;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
        options
    };
    #[cfg(windows)]
    let options = {
        use interprocess::os::windows::{
            local_socket::ListenerOptionsExt, security_descriptor::SecurityDescriptor,
        };
        // Protected DACL: only the object owner may open this named pipe.
        let descriptor = widestring::U16CString::from_str("D:P(A;;GA;;;OW)")?;
        options.security_descriptor(SecurityDescriptor::deserialize(&descriptor)?)
    };
    let listener = options.create_sync()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // The containing profile is 0700 before bind, so no other user can reach
        // the endpoint while its permissions are set (Darwin rejects fchmod on sockets).
        std::fs::set_permissions(
            data_dir.join("cli.sock"),
            std::fs::Permissions::from_mode(0o600),
        )?;
    }
    Ok(listener)
}

pub(super) fn verify_peer(stream: &Stream, data_dir: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        anyhow::ensure!(
            stream.peer_creds()?.euid() == Some(std::fs::metadata(data_dir)?.uid()),
            "The Iris service must belong to the profile owner"
        );
    }
    #[cfg(windows)]
    {
        let _ = data_dir;
        let pid = stream
            .peer_creds()?
            .pid()
            .context("Service peer has no process identity")?;
        anyhow::ensure!(
            windows_same_user(pid)?,
            "The Iris service must belong to the current user"
        );
    }
    Ok(())
}

#[cfg(windows)]
fn windows_same_user(pid: u32) -> Result<bool> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        Security::{EqualSid, GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER},
        System::Threading::{
            GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
        },
    };
    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    fn token_user(process: HANDLE) -> Result<Vec<usize>> {
        let mut token = std::ptr::null_mut();
        anyhow::ensure!(
            unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } != 0,
            "Cannot authenticate local service peer"
        );
        let token = Handle(token);
        let mut len = 0;
        unsafe {
            GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut len);
        }
        anyhow::ensure!(len > 0, "Cannot read local service identity");
        let mut data = vec![0usize; (len as usize).div_ceil(std::mem::size_of::<usize>())];
        anyhow::ensure!(
            unsafe {
                GetTokenInformation(token.0, TokenUser, data.as_mut_ptr().cast(), len, &mut len)
            } != 0,
            "Cannot read local service identity"
        );
        Ok(data)
    }
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    anyhow::ensure!(
        !process.is_null(),
        "Cannot authenticate local service process"
    );
    let process = Handle(process);
    let ours = token_user(unsafe { GetCurrentProcess() })?;
    let theirs = token_user(process.0)?;
    // Both buffers contain a successful TokenUser result and stay alive through EqualSid.
    Ok(unsafe {
        EqualSid(
            (*(ours.as_ptr().cast::<TOKEN_USER>())).User.Sid,
            (*(theirs.as_ptr().cast::<TOKEN_USER>())).User.Sid,
        )
    } != 0)
}

pub(super) struct Connection {
    pub stream: Stream,
    input: Vec<u8>,
    output: Vec<u8>,
    written: usize,
    write_started: Instant,
}
impl Connection {
    pub fn new(stream: Stream) -> Result<Self> {
        stream.set_nonblocking(true)?;
        Ok(Self {
            stream,
            input: Vec::new(),
            output: Vec::new(),
            written: 0,
            write_started: Instant::now(),
        })
    }
    pub fn receive(&mut self, limit: usize) -> Result<Option<Value>> {
        let mut buf = [0u8; 8192];
        // Bounded work per poll, even if a client keeps writing without a delimiter.
        for _ in 0..8 {
            if let Some(end) = self.input.iter().position(|b| *b == b'\n') {
                let frame: Vec<u8> = self.input.drain(..=end).collect();
                anyhow::ensure!(frame.len() <= limit, "Service frame is too large");
                return Ok(Some(
                    serde_json::from_slice(&frame).context("Invalid service frame")?,
                ));
            }
            anyhow::ensure!(self.input.len() <= limit, "Service frame is too large");
            match read_available(&self.stream, &mut buf) {
                Ok(0) => anyhow::bail!("Iris service connection closed; do not retry a send without checking its result"),
                Ok(n) => self.input.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(None),
                Err(e) => return Err(e.into()),
            }
        }
        Ok(None)
    }
    pub fn queue(&mut self, value: &Value) -> Result<()> {
        anyhow::ensure!(
            self.output.len() < 32 * 1024 * 1024,
            "Service client is not reading responses"
        );
        if self.output.is_empty() {
            self.write_started = Instant::now();
        }
        serde_json::to_writer(&mut self.output, value)?;
        self.output.push(b'\n');
        Ok(())
    }
    pub fn flush(&mut self) -> Result<bool> {
        if self.output.is_empty() {
            return Ok(true);
        }
        anyhow::ensure!(
            self.write_started.elapsed() < Duration::from_secs(10),
            "Service client stopped reading"
        );
        // PIPE_NOWAIT writes larger than the pipe buffer can report zero bytes.
        // Keep each Windows write within interprocess's 512-byte buffer hint,
        // and bound work per tick so one response cannot starve other clients.
        for _ in 0..8 {
            #[cfg(windows)]
            let end = self.output.len().min(self.written + 512);
            #[cfg(not(windows))]
            let end = self.output.len();
            match (&self.stream).write(&self.output[self.written..end]) {
                #[cfg(windows)]
                Ok(0) => break,
                #[cfg(not(windows))]
                Ok(0) => anyhow::bail!("Service connection closed"),
                Ok(n) => {
                    self.written += n;
                    self.write_started = Instant::now();
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e.into()),
            }
            if self.written == self.output.len() {
                self.output.clear();
                self.written = 0;
                break;
            }
        }
        Ok(self.output.is_empty())
    }
}

fn read_available(stream: &Stream, buf: &mut [u8]) -> io::Result<usize> {
    #[cfg(windows)]
    {
        use std::os::windows::io::{AsHandle, AsRawHandle};
        use windows_sys::Win32::System::Pipes::PeekNamedPipe;
        let Stream::NamedPipe(pipe) = stream;
        let mut available = 0;
        // PIPE_NOWAIT returns ERROR_NO_DATA when idle, which std maps to
        // BrokenPipe. Peek first so an idle connection is not mistaken for EOF.
        let ok = unsafe {
            PeekNamedPipe(
                pipe.as_handle().as_raw_handle(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut available,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        if available == 0 {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let len = buf.len().min(available as usize);
        return (&*stream).read(&mut buf[..len]);
    }
    #[cfg(not(windows))]
    {
        (&*stream).read(buf)
    }
}
