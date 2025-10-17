//! Sandbox planning helpers.
//!
//! Defines high-level sandbox profiles and provides serializable plans that
//! callers can feed into lower-level runtimes (bwrap, firejail, …). The
//! implementation intentionally avoids performing any process execution.
//!
//! ## Prompt 9 Implementation
//!
//! This module implements sandbox plan generation as specified in Prompt 9:
//! - SandboxPlan with bind_ro, bind_rw, net, seccomp_profile fields
//! - plan_for_apply: strict repo RW, everything else RO, net=false
//! - plan_for_test: strict repo RW + tmp RW, permissive adds net=true
//! - No bwrap execution, just plan construction

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub use devit_common::SandboxProfile;

/// Standard read-only system paths for sandbox environments
const SYSTEM_RO_PATHS: &[&str] = &["/usr", "/bin", "/lib", "/lib64", "/etc", "/opt"];

/// Serializable sandbox plan capturing bind mounts, network access, and seccomp
/// configuration that should be applied by sandbox backends.
///
/// This structure follows the exact specification from Prompt 9 with fields:
/// bind_ro, bind_rw, net, seccomp_profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxPlan {
    /// Paths to mount read-only inside the sandbox
    pub bind_ro: Vec<PathBuf>,
    /// Paths that remain writable for the confined process
    pub bind_rw: Vec<PathBuf>,
    /// Whether outbound network access is permitted
    pub net: bool,
    /// Optional seccomp profile name for syscall filtering
    pub seccomp_profile: Option<String>,
}

impl SandboxPlan {
    /// Generates a sandbox plan for patch application workflows.
    ///
    /// **Strict profile**: Repository RW, everything else RO, net=false
    /// - bind_ro: System paths (/usr, /bin, /lib, etc.)
    /// - bind_rw: Repository root only
    /// - net: false (no network access)
    /// - seccomp_profile: "strict" for syscall filtering
    ///
    /// # Arguments
    /// * `repo_root` - Path to the repository root directory
    /// * `profile` - Sandbox profile (Strict or Permissive)
    ///
    /// # Returns
    /// Configured SandboxPlan for apply operations
    pub fn plan_for_apply(repo_root: PathBuf, profile: SandboxProfile) -> Self {
        let mut bind_ro = SYSTEM_RO_PATHS
            .iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        let bind_rw = vec![repo_root];

        // For strict profile, add additional RO paths for security
        if profile == SandboxProfile::Strict {
            bind_ro.extend([
                PathBuf::from("/proc"),
                PathBuf::from("/sys"),
                PathBuf::from("/dev"),
            ]);
        }

        Self {
            bind_ro,
            bind_rw,
            net: false, // No network access for apply operations
            seccomp_profile: match profile {
                SandboxProfile::Strict => Some("strict".to_string()),
                SandboxProfile::Permissive => Some("permissive".to_string()),
            },
        }
    }

    /// Generates a sandbox plan for test execution workflows.
    ///
    /// **Strict profile**: Repository RW, tmp RW, net=false
    /// **Permissive profile**: Repository RW, tmp RW, net=true
    ///
    /// # Arguments
    /// * `repo_root` - Path to the repository root directory
    /// * `profile` - Sandbox profile determining network access and restrictions
    ///
    /// # Returns
    /// Configured SandboxPlan for test operations
    pub fn plan_for_test(repo_root: PathBuf, profile: SandboxProfile) -> Self {
        let bind_ro = SYSTEM_RO_PATHS
            .iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();

        let mut bind_rw = vec![repo_root, PathBuf::from("/tmp")];

        // Permissive profile gets additional writable paths
        if profile == SandboxProfile::Permissive {
            bind_rw.extend([PathBuf::from("/var/tmp"), PathBuf::from("/home")]);
        }

        Self {
            bind_ro,
            bind_rw,
            net: profile == SandboxProfile::Permissive, // Network only for permissive
            seccomp_profile: match profile {
                SandboxProfile::Strict => Some("strict".to_string()),
                SandboxProfile::Permissive => None, // No seccomp restrictions for permissive
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_for_apply_strict_creates_secure_environment() {
        let repo_root = PathBuf::from("/workspace/myproject");
        let plan = SandboxPlan::plan_for_apply(repo_root.clone(), SandboxProfile::Strict);

        // Verify bind mounts
        assert!(plan.bind_ro.contains(&PathBuf::from("/usr")));
        assert!(plan.bind_ro.contains(&PathBuf::from("/bin")));
        assert!(plan.bind_ro.contains(&PathBuf::from("/lib")));
        assert!(plan.bind_ro.contains(&PathBuf::from("/proc")));
        assert!(plan.bind_ro.contains(&PathBuf::from("/sys")));

        // Repository should be writable
        assert_eq!(plan.bind_rw, vec![repo_root]);

        // No network access for apply operations
        assert!(!plan.net);

        // Strict seccomp profile
        assert_eq!(plan.seccomp_profile, Some("strict".to_string()));
    }

    #[test]
    fn plan_for_apply_permissive_allows_more_access() {
        let repo_root = PathBuf::from("/workspace/myproject");
        let plan = SandboxPlan::plan_for_apply(repo_root.clone(), SandboxProfile::Permissive);

        // Still no network for apply operations
        assert!(!plan.net);

        // Repository should be writable
        assert_eq!(plan.bind_rw, vec![repo_root]);

        // Permissive seccomp profile
        assert_eq!(plan.seccomp_profile, Some("permissive".to_string()));
    }

    #[test]
    fn plan_for_test_strict_enables_testing_environment() {
        let repo_root = PathBuf::from("/workspace/myproject");
        let plan = SandboxPlan::plan_for_test(repo_root.clone(), SandboxProfile::Strict);

        // Repository and tmp should be writable
        assert!(plan.bind_rw.contains(&repo_root));
        assert!(plan.bind_rw.contains(&PathBuf::from("/tmp")));

        // No network access for strict testing
        assert!(!plan.net);

        // Strict seccomp profile
        assert_eq!(plan.seccomp_profile, Some("strict".to_string()));
    }

    #[test]
    fn plan_for_test_permissive_enables_network() {
        let repo_root = PathBuf::from("/workspace/myproject");
        let plan = SandboxPlan::plan_for_test(repo_root.clone(), SandboxProfile::Permissive);

        // Repository, tmp, and additional paths should be writable
        assert!(plan.bind_rw.contains(&repo_root));
        assert!(plan.bind_rw.contains(&PathBuf::from("/tmp")));
        assert!(plan.bind_rw.contains(&PathBuf::from("/var/tmp")));
        assert!(plan.bind_rw.contains(&PathBuf::from("/home")));

        // Network access enabled for permissive testing
        assert!(plan.net);

        // No seccomp restrictions for permissive
        assert_eq!(plan.seccomp_profile, None);
    }

    #[test]
    fn sandbox_plan_json_serialization_is_stable() {
        let repo_root = PathBuf::from("/workspace/test");
        let plan = SandboxPlan::plan_for_apply(repo_root, SandboxProfile::Strict);

        // Test serialization
        let json = serde_json::to_string(&plan).expect("serialize plan");
        assert!(json.contains("\"bind_ro\""));
        assert!(json.contains("\"bind_rw\""));
        assert!(json.contains("\"net\""));
        assert!(json.contains("\"seccomp_profile\""));

        // Test deserialization
        let deserialized: SandboxPlan = serde_json::from_str(&json).expect("deserialize plan");
        assert_eq!(plan, deserialized);
    }

    #[test]
    fn sandbox_plan_json_serialization_is_deterministic() {
        let repo_root = PathBuf::from("/workspace/test");
        let plan1 = SandboxPlan::plan_for_apply(repo_root.clone(), SandboxProfile::Strict);
        let plan2 = SandboxPlan::plan_for_apply(repo_root, SandboxProfile::Strict);

        let json1 = serde_json::to_string(&plan1).expect("serialize plan1");
        let json2 = serde_json::to_string(&plan2).expect("serialize plan2");

        // Same inputs should produce identical JSON
        assert_eq!(json1, json2);
    }

    #[test]
    fn strict_and_permissive_profiles_have_consistent_behavior() {
        let repo_root = PathBuf::from("/workspace/test");

        let strict_apply = SandboxPlan::plan_for_apply(repo_root.clone(), SandboxProfile::Strict);
        let permissive_apply =
            SandboxPlan::plan_for_apply(repo_root.clone(), SandboxProfile::Permissive);

        // Both apply plans should have no network access
        assert!(!strict_apply.net);
        assert!(!permissive_apply.net);

        let strict_test = SandboxPlan::plan_for_test(repo_root.clone(), SandboxProfile::Strict);
        let permissive_test = SandboxPlan::plan_for_test(repo_root, SandboxProfile::Permissive);

        // Only permissive test should have network access
        assert!(!strict_test.net);
        assert!(permissive_test.net);

        // Permissive should have more writable paths
        assert!(permissive_test.bind_rw.len() > strict_test.bind_rw.len());
    }
}
