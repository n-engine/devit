use std::process::Command;

/// Returns a platform-appropriate shell command to execute `script`.
/// - Unix: `bash -c <script>`
/// - Windows: `cmd /C <script>`
pub fn shell_command(script: &str) -> Command {
    #[cfg(unix)]
    {
        let mut cmd = Command::new("bash");
        cmd.arg("-c").arg(script);
        cmd
    }
    #[cfg(windows)]
    {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(script);
        cmd
    }
}
