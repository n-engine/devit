use std::io;
use std::path::Path;

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{UnixListener, UnixStream};

use super::{IpcBackend, IpcListener};

pub struct UnixIpcBackend;

pub struct UnixIpcListener {
    inner: UnixListener,
}

#[async_trait]
impl IpcBackend for UnixIpcBackend {
    type Stream = UnixStream;
    type Listener = UnixIpcListener;

    async fn bind(addr: &str) -> io::Result<Self::Listener> {
        // Ensure parent directory exists when using filesystem paths
        if let Some(parent) = Path::new(addr).parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let _ = std::fs::remove_file(addr);
        let listener = UnixListener::bind(addr)?;
        Ok(UnixIpcListener { inner: listener })
    }

    async fn connect(addr: &str) -> io::Result<Self::Stream> {
        UnixStream::connect(addr).await
    }
}

#[async_trait]
impl IpcListener for UnixIpcListener {
    type Stream = UnixStream;

    async fn accept(&mut self) -> io::Result<Self::Stream> {
        let (stream, _addr) = self.inner.accept().await?;
        Ok(stream)
    }
}

