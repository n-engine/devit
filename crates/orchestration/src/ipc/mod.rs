use std::io;

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite};

#[async_trait]
pub trait IpcBackend: Send + Sync + 'static {
    type Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static;
    type Listener: IpcListener<Stream = Self::Stream> + Send + 'static;

    async fn bind(addr: &str) -> io::Result<Self::Listener>;
    async fn connect(addr: &str) -> io::Result<Self::Stream>;
}

#[async_trait]
pub trait IpcListener: Send {
    type Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static;
    async fn accept(&mut self) -> io::Result<Self::Stream>;
}

#[cfg(unix)]
pub mod unix;

#[cfg(windows)]
pub mod windows;

