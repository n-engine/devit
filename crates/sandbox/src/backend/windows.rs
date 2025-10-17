use super::{ProcessHandle, SandboxBackend};
use anyhow::Result;
use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus};

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::JobObjects::*;

#[derive(Debug)]
pub struct WindowsSandbox {
    job: OwnedHandle,
    cpu_limit: Option<u32>,
    memory_limit: Option<u64>,
}

impl WindowsSandbox {
    pub fn new() -> io::Result<Self> {
        let raw_job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw_job.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = unsafe { OwnedHandle::from_raw_handle(raw_job as RawHandle) };
        Ok(Self {
            job,
            cpu_limit: None,
            memory_limit: None,
        })
    }

    fn apply_cpu_limit(&self, percent: u32) -> io::Result<()> {
        // CPU rate control: 10000 = 100%
        let rate = (percent as u32) * 100;
        let mut info: JOBOBJECT_CPU_RATE_CONTROL_INFORMATION = unsafe { zeroed() };
        info.ControlFlags =
            JOB_OBJECT_CPU_RATE_CONTROL_ENABLE | JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP;
        info.Anonymous = JOBOBJECT_CPU_RATE_CONTROL_INFORMATION_0 { CpuRate: rate };

        let job_handle = self.job.as_raw_handle() as HANDLE;

        let result = unsafe {
            SetInformationJobObject(
                job_handle,
                JobObjectCpuRateControlInformation,
                &mut info as *mut _ as *mut _,
                size_of::<JOBOBJECT_CPU_RATE_CONTROL_INFORMATION>() as u32,
            )
        };
        if result == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn apply_memory_limit(&self, bytes: u64) -> io::Result<()> {
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_PROCESS_MEMORY;
        info.ProcessMemoryLimit = bytes as usize;

        let job_handle = self.job.as_raw_handle() as HANDLE;

        let result = unsafe {
            SetInformationJobObject(
                job_handle,
                JobObjectExtendedLimitInformation,
                &mut info as *mut _ as *mut _,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if result == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

impl SandboxBackend for WindowsSandbox {
    type Child = WindowsChild;

    fn spawn(&mut self, mut cmd: Command) -> Result<Self::Child> {
        let child = cmd.spawn()?;

        // Assign spawned process to job
        let process_handle = child.as_raw_handle() as HANDLE;
        let job_handle = self.job.as_raw_handle() as HANDLE;
        let result = unsafe { AssignProcessToJobObject(job_handle, process_handle) };
        if result == 0 {
            return Err(io::Error::last_os_error().into());
        }

        if let Some(cpu) = self.cpu_limit {
            self.apply_cpu_limit(cpu)?;
        }
        if let Some(mem) = self.memory_limit {
            self.apply_memory_limit(mem)?;
        }

        // Duplicate the job handle so the child keeps the job alive even if the sandbox is dropped.
        let child_job = self.job.try_clone()?;

        Ok(WindowsChild {
            job: child_job,
            child,
            kill_on_drop: true,
        })
    }

    fn set_cpu_limit(&mut self, percent: u32) -> Result<()> {
        self.cpu_limit = Some(percent);
        Ok(())
    }

    fn set_memory_limit(&mut self, bytes: u64) -> Result<()> {
        self.memory_limit = Some(bytes);
        Ok(())
    }
}

pub struct WindowsChild {
    pub(crate) job: OwnedHandle,
    pub(crate) child: Child,
    kill_on_drop: bool,
}

impl WindowsChild {
    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout.take()
    }

    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.stderr.take()
    }

    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.stdin.take()
    }

    pub fn disable_kill_on_drop(&mut self) {
        self.kill_on_drop = false;
    }
}

impl ProcessHandle for WindowsChild {
    fn id(&self) -> u32 {
        self.child.id()
    }

    fn kill(&mut self) -> io::Result<()> {
        let job_handle = self.job.as_raw_handle() as HANDLE;
        let terminated = unsafe { TerminateJobObject(job_handle, 1) };
        if terminated == 0 {
            // Fallback to killing the root process if terminating the job failed
            self.child.kill()
        } else {
            Ok(())
        }
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        self.child.wait()
    }

    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    fn stdout(&mut self) -> Option<&mut ChildStdout> {
        self.child.stdout.as_mut()
    }

    fn stderr(&mut self) -> Option<&mut ChildStderr> {
        self.child.stderr.as_mut()
    }

    fn stdin(&mut self) -> Option<&mut ChildStdin> {
        self.child.stdin.as_mut()
    }
}

impl Drop for WindowsChild {
    fn drop(&mut self) {
        if self.kill_on_drop && self.child.try_wait().ok().flatten().is_none() {
            let job_handle = self.job.as_raw_handle() as HANDLE;
            unsafe {
                TerminateJobObject(job_handle, 1);
            }
        }
        // OwnedHandle drop will close the handle automatically.
    }
}
