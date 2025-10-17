use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemCapabilities {
    pub sandbox: SandboxCapabilities,
    pub vcs: VcsCapabilities,
    pub limits: SystemLimits,
    pub sandbox_profiles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxCapabilities {
    pub bwrap_available: bool,
    pub bwrap_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VcsCapabilities {
    pub git_available: bool,
    pub git_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemLimits {
    pub cpu_count: Option<u32>,
    pub memory_total_mb: Option<u64>,
    pub max_open_files: Option<u64>,
}

impl SystemCapabilities {
    pub fn detect() -> Result<Self> {
        Ok(Self {
            sandbox: SandboxCapabilities::detect()?,
            vcs: VcsCapabilities::detect()?,
            limits: SystemLimits::detect()?,
            sandbox_profiles: get_supported_sandbox_profiles(),
        })
    }
}

impl SandboxCapabilities {
    pub fn detect() -> Result<Self> {
        let (bwrap_available, bwrap_version) = detect_bwrap()?;

        Ok(Self {
            bwrap_available,
            bwrap_version,
        })
    }
}

impl VcsCapabilities {
    pub fn detect() -> Result<Self> {
        let (git_available, git_version) = detect_git()?;

        Ok(Self {
            git_available,
            git_version,
        })
    }
}

impl SystemLimits {
    pub fn detect() -> Result<Self> {
        Ok(Self {
            cpu_count: detect_cpu_count(),
            memory_total_mb: detect_memory_total_mb(),
            max_open_files: detect_max_open_files(),
        })
    }
}

fn detect_bwrap() -> Result<(bool, Option<String>)> {
    match Command::new("bwrap").arg("--version").output() {
        Ok(output) if output.status.success() => {
            let version_output = String::from_utf8_lossy(&output.stdout);
            let version = parse_bwrap_version(&version_output);
            Ok((true, version))
        }
        _ => Ok((false, None)),
    }
}

fn parse_bwrap_version(output: &str) -> Option<String> {
    // Example output: "bubblewrap 0.4.1"
    output
        .lines()
        .next()?
        .split_whitespace()
        .nth(1)
        .map(|v| v.to_string())
}

fn detect_git() -> Result<(bool, Option<String>)> {
    match Command::new("git").arg("--version").output() {
        Ok(output) if output.status.success() => {
            let version_output = String::from_utf8_lossy(&output.stdout);
            let version = parse_git_version(&version_output);
            Ok((true, version))
        }
        _ => Ok((false, None)),
    }
}

fn parse_git_version(output: &str) -> Option<String> {
    // Example output: "git version 2.34.1"
    output
        .lines()
        .next()?
        .split_whitespace()
        .nth(2)
        .map(|v| v.to_string())
}

fn detect_cpu_count() -> Option<u32> {
    std::thread::available_parallelism()
        .ok()
        .map(|n| n.get() as u32)
}

fn detect_memory_total_mb() -> Option<u64> {
    // Best effort: try to read from /proc/meminfo on Linux
    #[cfg(target_os = "linux")]
    {
        use std::fs;
        let content = fs::read_to_string("/proc/meminfo").ok()?;
        for line in content.lines() {
            if line.starts_with("MemTotal:") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let Ok(kb) = parts[1].parse::<u64>() {
                        return Some(kb / 1024); // Convert KB to MB
                    }
                }
            }
        }
    }

    None
}

fn detect_max_open_files() -> Option<u64> {
    // Best effort: try to get from ulimit
    match Command::new("sh").arg("-c").arg("ulimit -n").output() {
        Ok(output) if output.status.success() => {
            let output_str = String::from_utf8_lossy(&output.stdout);
            output_str.trim().parse().ok()
        }
        _ => None,
    }
}

fn get_supported_sandbox_profiles() -> Vec<String> {
    let mut profiles = vec!["read_only".to_string(), "workspace_write".to_string()];

    // Add bwrap profile if bwrap is available
    if detect_bwrap().unwrap_or((false, None)).0 {
        profiles.push("bwrap".to_string());
    }

    // Always available
    profiles.push("danger_full_access".to_string());

    profiles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bwrap_version() {
        assert_eq!(
            parse_bwrap_version("bubblewrap 0.4.1"),
            Some("0.4.1".to_string())
        );
        assert_eq!(parse_bwrap_version("invalid"), None);
    }

    #[test]
    fn test_parse_git_version() {
        assert_eq!(
            parse_git_version("git version 2.34.1"),
            Some("2.34.1".to_string())
        );
        assert_eq!(parse_git_version("invalid"), None);
    }

    #[test]
    fn test_system_capabilities_detect() {
        let capabilities = SystemCapabilities::detect();
        assert!(capabilities.is_ok());

        let caps = capabilities.unwrap();
        assert!(!caps.sandbox_profiles.is_empty());
        assert!(caps.sandbox_profiles.contains(&"read_only".to_string()));
        assert!(caps
            .sandbox_profiles
            .contains(&"workspace_write".to_string()));
    }

    #[test]
    fn test_cpu_count_detection() {
        let cpu_count = detect_cpu_count();
        assert!(cpu_count.is_some());
        assert!(cpu_count.unwrap() > 0);
    }

    #[test]
    fn test_sandbox_capabilities_has_required_fields() {
        let caps = SandboxCapabilities::detect().unwrap();
        // bwrap_available should always be a boolean
        assert!(caps.bwrap_available || !caps.bwrap_available); // Always true but explicit test

        // If bwrap is available, version should be present
        if caps.bwrap_available {
            assert!(caps.bwrap_version.is_some());
            let version = caps.bwrap_version.unwrap();
            assert!(!version.is_empty());
            // Should look like a version number (contains a dot)
            assert!(version.contains('.'));
        }
    }

    #[test]
    fn test_vcs_capabilities_has_required_fields() {
        let caps = VcsCapabilities::detect().unwrap();
        // git_available should always be a boolean
        assert!(caps.git_available || !caps.git_available);

        // If git is available, version should be present
        if caps.git_available {
            assert!(caps.git_version.is_some());
            let version = caps.git_version.unwrap();
            assert!(!version.is_empty());
            // Should look like a version number (contains a dot)
            assert!(version.contains('.'));
        }
    }

    #[test]
    fn test_system_limits_structure() {
        let limits = SystemLimits::detect().unwrap();

        // CPU count should always be detected on most systems
        if let Some(cpu_count) = limits.cpu_count {
            assert!(cpu_count > 0);
            assert!(cpu_count <= 1024); // Reasonable upper bound
        }

        // Memory should be detected on most systems
        if let Some(memory_mb) = limits.memory_total_mb {
            assert!(memory_mb > 0);
            assert!(memory_mb < 1_000_000); // Less than 1TB
        }

        // Max open files should be detected on Unix systems
        if let Some(max_files) = limits.max_open_files {
            assert!(max_files > 0);
        }
    }

    #[test]
    fn test_sandbox_profiles_completeness() {
        let profiles = get_supported_sandbox_profiles();

        // Should always include basic profiles
        assert!(profiles.contains(&"read_only".to_string()));
        assert!(profiles.contains(&"workspace_write".to_string()));
        assert!(profiles.contains(&"danger_full_access".to_string()));

        // Should be non-empty
        assert!(!profiles.is_empty());
        assert!(profiles.len() >= 3);
    }

    #[test]
    fn test_capabilities_json_serialization() {
        let caps = SystemCapabilities::detect().unwrap();

        // Should serialize to JSON without error
        let json_result = serde_json::to_string(&caps);
        assert!(json_result.is_ok());

        let json_str = json_result.unwrap();
        assert!(!json_str.is_empty());

        // Should deserialize back
        let deserialized: Result<SystemCapabilities, _> = serde_json::from_str(&json_str);
        assert!(deserialized.is_ok());

        let deserialized_caps = deserialized.unwrap();
        assert_eq!(
            caps.sandbox_profiles.len(),
            deserialized_caps.sandbox_profiles.len()
        );
    }
}
