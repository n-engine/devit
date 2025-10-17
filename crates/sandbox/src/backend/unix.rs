use super::{ProcessHandle, SandboxBackend};
use anyhow::Result;
use std::io;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus};

#[derive(Debug, Default, Clone)]
pub struct UnixSandbox {
    cpu_limit_percent: Option<u32>,
    memory_limit_bytes: Option<u64>,
}

impl SandboxBackend for UnixSandbox {
    type Child = UnixChild;

    fn spawn(&mut self, mut cmd: Command) -> Result<Self::Child> {
        // Phase 1B: no runtime resource enforcement yet; spawn directly.
        // Callers should set stdio as needed on the Command before passing it here.
        let child = cmd.spawn()?;
        Ok(UnixChild(child))
    }

    fn set_cpu_limit(&mut self, percent: u32) -> Result<()> {
        self.cpu_limit_percent = Some(percent);
        Ok(())
    }

    fn set_memory_limit(&mut self, bytes: u64) -> Result<()> {
        self.memory_limit_bytes = Some(bytes);
        Ok(())
    }
}

pub struct UnixChild(pub(crate) Child);

impl ProcessHandle for UnixChild {
    fn id(&self) -> u32 {
        self.0.id()
    }

    fn kill(&mut self) -> io::Result<()> {
        self.0.kill()
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        self.0.wait()
    }

    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.0.try_wait()
    }

    fn stdout(&mut self) -> Option<&mut ChildStdout> {
        self.0.stdout.as_mut()
    }

    fn stderr(&mut self) -> Option<&mut ChildStderr> {
        self.0.stderr.as_mut()
    }

    fn stdin(&mut self) -> Option<&mut ChildStdin> {
        self.0.stdin.as_mut()
    }
}
