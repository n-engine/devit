use std::io;

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions};

use super::{IpcBackend, IpcListener};

// Windows Named Pipes backend using tokio's async named pipe support.
pub struct WindowsIpcBackend;

pub struct WindowsIpcListener {
    name: String,
    server: NamedPipeServer,
}

// Single stream type that can represent either a server-side accepted pipe
// or a client-side connected pipe.
pub enum WindowsPipeStream {
    Server(NamedPipeServer),
    Client(NamedPipeClient),
}

impl Unpin for WindowsPipeStream {}

impl tokio::io::AsyncRead for WindowsPipeStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        match self.get_mut() {
            WindowsPipeStream::Server(s) => std::pin::Pin::new(s).poll_read(cx, buf),
            WindowsPipeStream::Client(c) => std::pin::Pin::new(c).poll_read(cx, buf),
        }
    }
}

impl tokio::io::AsyncWrite for WindowsPipeStream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        match self.get_mut() {
            WindowsPipeStream::Server(s) => std::pin::Pin::new(s).poll_write(cx, buf),
            WindowsPipeStream::Client(c) => std::pin::Pin::new(c).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        match self.get_mut() {
            WindowsPipeStream::Server(s) => std::pin::Pin::new(s).poll_flush(cx),
            WindowsPipeStream::Client(c) => std::pin::Pin::new(c).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        match self.get_mut() {
            WindowsPipeStream::Server(s) => std::pin::Pin::new(s).poll_shutdown(cx),
            WindowsPipeStream::Client(c) => std::pin::Pin::new(c).poll_shutdown(cx),
        }
    }
}

#[async_trait]
impl IpcBackend for WindowsIpcBackend {
    type Stream = WindowsPipeStream;
    type Listener = WindowsIpcListener;

    async fn bind(addr: &str) -> io::Result<Self::Listener> {
        let name = normalize_pipe_name(addr);
        let server = ServerOptions::new().create(&name)?;
        Ok(WindowsIpcListener { name, server })
    }

    async fn connect(addr: &str) -> io::Result<Self::Stream> {
        let name = normalize_pipe_name(addr);
        // ClientOptions::open is sync; fine to call inside async fn.
        let client = ClientOptions::new().open(&name)?;
        Ok(WindowsPipeStream::Client(client))
    }
}

#[async_trait]
impl IpcListener for WindowsIpcListener {
    type Stream = WindowsPipeStream;

    async fn accept(&mut self) -> io::Result<Self::Stream> {
        // Wait for a client to connect to the current server instance
        self.server.connect().await?;

        // Swap out the connected instance to return it, and create a fresh
        // server instance for the next connection.
        let connected = std::mem::replace(
            &mut self.server,
            ServerOptions::new().create(&self.name)?,
        );

        Ok(WindowsPipeStream::Server(connected))
    }
}

fn normalize_pipe_name(addr: &str) -> String {
    let trimmed = addr.trim();
    if trimmed.starts_with("\\\\.\\pipe\\") || trimmed.starts_with(r"\\.\pipe\") {
        // Already a full pipe path (handles both escaped and raw string styles)
        trimmed.to_string()
    } else if trimmed.starts_with("pipe:") {
        // Allow a scheme-like form: pipe:devitd
        let name = &trimmed[5..];
        format!(r"\\.\pipe\{}", name)
    } else {
        // Treat as bare pipe name
        format!(r"\\.\pipe\{}", trimmed)
    }
}
