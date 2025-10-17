use anyhow::Result;
use std::io;
use std::process::{ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus};

pub trait SandboxBackend: Send {
    type Child: ProcessHandle;

    fn spawn(&mut self, cmd: Command) -> Result<Self::Child>;
    fn set_cpu_limit(&mut self, percent: u32) -> Result<()>;
    fn set_memory_limit(&mut self, bytes: u64) -> Result<()>;
}

pub trait ProcessHandle: Send {
    fn id(&self) -> u32;
    fn kill(&mut self) -> io::Result<()>;
    fn wait(&mut self) -> io::Result<ExitStatus>;
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>>;

    fn stdout(&mut self) -> Option<&mut ChildStdout>;
    fn stderr(&mut self) -> Option<&mut ChildStderr>;
    fn stdin(&mut self) -> Option<&mut ChildStdin>;
}

#[cfg(unix)]
pub mod unix;

#[cfg(windows)]
pub mod windows;
